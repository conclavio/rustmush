//! The `HousingModule` (E5b, 08d §4) — the cut that attaches E5a's pure library to
//! a running world.
//!
//! ## What lives here
//!
//! The module state slot ([`HousingStore`] — every property, every tenancy, the two
//! never-reused allocators, the per-tenancy assessment cursors and the bounded
//! pending-notice lists), the persistence keys, and the registration that wires
//! everything else up: the `MoveAttempt`/`CarryAttempt` hook ([`crate::hooks`]), the
//! destroy gate and the `ObjDestroyed` backstop, the `Connected` notice, the
//! cadence-driven assessment tick ([`crate::tick`]), the exact-mode economy observer
//! ([`crate::econ`]), the `wod-story` job queue, and the command surface
//! ([`crate::commands`]).
//!
//! ## Contracts (08d §4, and the E5a fold's handoffs in §3.5)
//!
//! - **`property:{id}` and `tenancy:{id}` are container docs.** Their state *is* the
//!   record — who lives where, and which rooms the access hook answers for — so a
//!   corrupt one **fails the boot** rather than booting a world in which a home
//!   quietly does not exist. Both deserialize through E5a's `TryFrom` wires, so every
//!   §9 cap is re-checked on load. `assess:{id}` and `notice:{obj}` are **leaf** docs
//!   (a cursor and a message queue): a corrupt one is skipped loudly and re-seeded.
//! - **Allocators are floored on keys *and* meta, saturating** (the E3/E4b1 rule): a
//!   single hand-written `property:18446744073709551615` must park the allocator at
//!   the ceiling, where minting refuses, rather than wrapping a live id back into use.
//! - **One live tenancy per property, picked explicitly** (§3.5's first handoff).
//!   [`HousingStore::live_tenancy_of`] is the only place that answers "which tenancy
//!   does this room's access question belong to", it refuses to guess between two,
//!   and `@tenancy/create` refuses to make a second.
//! - **No housing code moves money.** Rent is an E4 obligation under an E4 contract;
//!   this crate stores two ids and reads the obligation's status.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{
    GameModule, JobQueueSpec, ModError, ModuleBootData, ModuleCtx, Registrar, TickCadence,
    WorldEventKind,
};
use mush_core::{Ctx, Flags, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox, Role, World};
use mush_org::{PrincipalId, SystemPrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::{PropertyId, TenancyId};
use crate::record::{Property, PropertyStatus, Tenancy};

/// The module id and persistence namespace.
pub const NS: &str = "housing";
/// The kv key of the allocator meta document.
pub const META_KEY: &str = "meta";
/// The staff/ST job queue a story window opens into (08d §4).
pub const STORY_QUEUE: &str = "wod-story";
/// How often the assessment tick fires (wall clock). It is only a *latency* bound:
/// the tick is a **pull**, comparing each tenancy's cadence position against the one
/// it last assessed, so a fire that finds nothing moved does nothing and a fire that
/// misses a period picks it up on the next one.
pub const TICK_PERIOD_SECS: u64 = 30;
/// The tick's tag.
pub const TICK_TAG: &str = "housing.assess";

/// The most tenancies one tick fire assesses (08d §9). The remainder is taken by the
/// next fire, from the rotation cursor, so a world with more tenancies than this
/// still reaches every one of them.
pub const MAX_ASSESSMENTS_PER_FIRE: usize = 128;
/// The most pending notices one object queues before the oldest is dropped (§9).
pub const MAX_PENDING_NOTICES: usize = 32;
/// The largest single notice text a pending list stores (the bounded-string
/// discipline `mush-economy` applies to every stored string).
pub const MAX_NOTICE_BYTES: usize = 240;

/// The marker a builder sets on a room that must never become part of a property
/// (`@set <room>=no-property`). Housing registers it, so `@set` accepts it and
/// `@property/rooms` refuses the room without a config change or a `mush-sim`
/// dependency housing is not allowed to have (08d §2).
pub const NO_PROPERTY_MARKER: &str = "no-property";

/// The capability vocabulary housing asks `mush-org` about (08d §4, §11 answer 3).
///
/// `property.view` is a **landlord officer's read** — their own compartment's
/// properties and the tenancies on them. `property.manage` covers show/list and the
/// guest-policy edits, and deliberately **not** tenancy creation: §11's third answer
/// is binding, and creating a tenancy is staff-only in E5.
pub const CAPABILITIES: &[&str] = &["property.view", "property.manage"];

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// `property:{id}` — a **container** doc: a corrupt one fails the boot.
#[must_use]
pub fn property_key(id: PropertyId) -> SmolStr {
    SmolStr::new(format!("property:{}", id.0))
}
/// `tenancy:{id}` — a **container** doc: a corrupt one fails the boot.
#[must_use]
pub fn tenancy_key(id: TenancyId) -> SmolStr {
    SmolStr::new(format!("tenancy:{}", id.0))
}
/// `assess:{id}` — a **leaf** doc (the pull-model cursor): a corrupt one is skipped
/// loudly and re-seeded from the tenancy's clock at the next fire, which costs at
/// most one unassessed period and never a wrong one.
#[must_use]
pub fn assess_key(id: TenancyId) -> SmolStr {
    SmolStr::new(format!("assess:{}", id.0))
}
/// `notice:{obj}` — a **leaf** doc (the offline queue): a corrupt one is skipped
/// loudly, which costs a player a notice they can still read off `+home`.
#[must_use]
pub fn notice_key(who: ObjId) -> SmolStr {
    SmolStr::new(format!("notice:{}", who.0))
}

// ---------------------------------------------------------------------------
// Per-tenancy assessment state (the pull model, 08d §4)
// ---------------------------------------------------------------------------

/// What the assessment tick remembers about one tenancy between fires.
///
/// It is module state and not tenancy-record state on purpose (E5a deviation 3): the
/// `Unknown` run counter and the period ordinal are *how this world has been running
/// the clock*, not facts about who lives where, and a record that carried them would
/// have to be rewritten on every silent supported period.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssessState {
    /// The cadence `position` this tenancy was last assessed at. The tick assesses
    /// only when the clock has moved past it — that is the whole pull model.
    #[serde(default)]
    pub position: i64,
    /// The monotonic period ordinal plan freshness is measured in
    /// ([`crate::PLAN_MAX_AGE_PERIODS`]). Separate from `position` because a
    /// re-pointed or restored cadence moves the position and must not make every
    /// plan look fresh again.
    #[serde(default)]
    pub period: u64,
    /// How many consecutive periods have assessed `Unknown`, including the current
    /// one — what [`crate::harden`] takes.
    #[serde(default)]
    pub unknown_periods: u32,
    /// Whether `position` has ever been set. A tenancy created between two fires must
    /// **not** read as "last assessed at position 0" on a clock that is at 900.
    #[serde(default)]
    pub seeded: bool,
}

/// One queued notice for an object that was not connected when it was sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingNotice {
    /// Wall clock, for the render.
    pub at: i64,
    /// The text (≤ [`MAX_NOTICE_BYTES`]).
    pub text: SmolStr,
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// The in-RAM housing slot — the single source of truth at runtime.
#[derive(Clone, Debug, Default)]
pub struct HousingStore {
    pub(crate) properties: BTreeMap<PropertyId, Property>,
    pub(crate) tenancies: BTreeMap<TenancyId, Tenancy>,
    /// The ids of every tenancy that has **not ended** — the tick's whole working
    /// set (verify fold, T7).
    ///
    /// A tenancy record is the largest document this module holds: a roster, a guest
    /// list, a plan per resident and a 32-entry history tail. Ended ones are kept
    /// forever (they are the audit record of who lived where), so on any world that
    /// has been running a while they are *most* of the store — and the tick was
    /// cloning every one of them, thirty seconds at a time, to discover that they
    /// were ended. This index is derived, never authoritative: [`put_tenancy`] and
    /// boot both re-derive it from the status, so it cannot drift into admitting an
    /// ended tenancy.
    pub(crate) open: BTreeSet<TenancyId>,
    pub(crate) next_property: u64,
    pub(crate) next_tenancy: u64,
    pub(crate) assess: BTreeMap<TenancyId, AssessState>,
    pub(crate) notices: BTreeMap<ObjId, Vec<PendingNotice>>,
    /// Where the assessment tick resumes next fire (E4b2's B4 rotation): the per-fire
    /// cap bounds *work*, and id order alone would starve the high-id tenancies of
    /// every fire, forever.
    pub(crate) cursor: u64,
    /// The title issuer this server wired (`with_issuer`). `None` means no property
    /// creation can mint title — the fail-closed default.
    pub(crate) issuer: Option<SystemPrincipalId>,
    /// Rooms this server reserved at wiring — the chargen start rooms the host parses
    /// out of `[onboarding]`, and anything else it knows must never be somebody's
    /// living room. Frozen at wiring like every other table in the E-series.
    pub(crate) reserved: BTreeSet<ObjId>,
}

impl HousingStore {
    /// One property.
    #[must_use]
    pub fn property(&self, id: PropertyId) -> Option<&Property> {
        self.properties.get(&id)
    }
    /// Every property, in id order.
    pub fn properties(&self) -> impl Iterator<Item = &Property> {
        self.properties.values()
    }
    /// One tenancy.
    #[must_use]
    pub fn tenancy(&self, id: TenancyId) -> Option<&Tenancy> {
        self.tenancies.get(&id)
    }
    /// Every tenancy, in id order.
    pub fn tenancies(&self) -> impl Iterator<Item = &Tenancy> {
        self.tenancies.values()
    }
    /// Every tenancy that has **not ended**, in id order — the ids the assessment
    /// tick walks (fold T7). Ended tenancies are kept forever as the record of who
    /// lived where, and nothing periodic ever needs to look at one again.
    pub fn open_tenancy_ids(&self) -> impl Iterator<Item = TenancyId> + '_ {
        self.open.iter().copied()
    }
    /// Re-derive [`HousingStore::open`] from the loaded tenancies (boot).
    pub(crate) fn reindex_open(&mut self) {
        self.open =
            self.tenancies.values().filter(|t| !t.status.is_ended()).map(|t| t.id).collect();
    }
    /// The wired title issuer, if this server has one.
    #[must_use]
    pub fn issuer(&self) -> Option<&SystemPrincipalId> {
        self.issuer.as_ref()
    }
    /// Whether this server reserved `room` against ever being a property room.
    #[must_use]
    pub fn is_reserved(&self, room: ObjId) -> bool {
        self.reserved.contains(&room)
    }

    /// Mint the next property id, or `None` at the ceiling (E4b1's B2 rule: an
    /// exhausted allocator refuses one operation loudly, it never wraps a live id).
    pub fn alloc_property(&mut self) -> Option<PropertyId> {
        let id = PropertyId(self.next_property);
        self.next_property = self.next_property.checked_add(1)?;
        Some(id)
    }
    /// Mint the next tenancy id, or `None` at the ceiling.
    pub fn alloc_tenancy(&mut self) -> Option<TenancyId> {
        let id = TenancyId(self.next_tenancy);
        self.next_tenancy = self.next_tenancy.checked_add(1)?;
        Some(id)
    }

    /// **The one live tenancy of a property** (E5a fold, handoff 1).
    ///
    /// The access hook, the destroy gate and every render ask this, and it must never
    /// be "any live tenancy on this room admits": two live tenancies would mean two
    /// rosters, two guest lists and two answers to the same door. `@tenancy/create`
    /// refuses to make a second, so the ambiguity is unreachable — and if a
    /// half-migrated document produces one anyway, this picks the **lowest id**
    /// deterministically and says so loudly, rather than letting the answer depend on
    /// iteration order.
    #[must_use]
    pub fn live_tenancy_of(&self, property: PropertyId) -> Option<&Tenancy> {
        let mut live = self
            .tenancies
            .values()
            .filter(|t| t.property == property && t.status.is_live());
        let first = live.next()?;
        if live.next().is_some() {
            tracing::error!(
                property = property.0,
                tenancy = first.id.0,
                "housing: a property carries more than one live tenancy; answering for the \
                 lowest id. End the others (@tenancy/resolve) — the invariant is one."
            );
        }
        Some(first)
    }

    /// Whether `property` already has a live tenancy (the invariant
    /// `@tenancy/create` enforces).
    #[must_use]
    pub fn has_live_tenancy(&self, property: PropertyId) -> bool {
        self.tenancies.values().any(|t| t.property == property && t.status.is_live())
    }

    /// The property one room belongs to, if any. A room belongs to at most one
    /// property (`@property/rooms` refuses a room another property already holds), so
    /// this is a total answer.
    #[must_use]
    pub fn property_of_room(&self, room: ObjId) -> Option<&Property> {
        self.properties.values().find(|p| p.rooms.contains(&room))
    }

    /// Every live tenancy this object is a tenant of.
    pub fn live_tenancies_of_tenant(&self, who: ObjId) -> impl Iterator<Item = &Tenancy> {
        self.tenancies.values().filter(move |t| t.status.is_live() && t.is_tenant(who))
    }

    /// The per-tenancy assessment cursor.
    #[must_use]
    pub fn assess_state(&self, id: TenancyId) -> AssessState {
        self.assess.get(&id).copied().unwrap_or_default()
    }

    /// The allocator resume document — high-water marks boot never lowers, plus the
    /// tick's rotation cursor.
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_property_id": self.next_property,
            "next_tenancy_id": self.next_tenancy,
            "assess_cursor": self.cursor,
        })
    }
}

// ---------------------------------------------------------------------------
// Persist helpers
// ---------------------------------------------------------------------------

pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp { ns: SmolStr::new_static(NS), op }));
}

/// Install a property into RAM and queue its container doc. Both always happen
/// together — there is no path that writes one without the other.
pub(crate) fn put_property(ctx: &mut Ctx<'_>, p: Property) {
    if let Some(store) = ctx.w.module_state_mut::<HousingStore>() {
        store.properties.insert(p.id, p.clone());
    }
    push_op(ctx.out, ModuleWrite::JsonSet {
        key: property_key(p.id),
        value: serde_json::to_value(&p).expect("property serializes"),
    });
}

/// Install a tenancy into RAM and queue its container doc.
pub(crate) fn put_tenancy(ctx: &mut Ctx<'_>, t: Tenancy) {
    if let Some(store) = ctx.w.module_state_mut::<HousingStore>() {
        // The open index is re-derived from the status on every write, so it says
        // exactly what the record says and no caller has to remember to maintain it.
        if t.status.is_ended() {
            store.open.remove(&t.id);
        } else {
            store.open.insert(t.id);
        }
        store.tenancies.insert(t.id, t.clone());
    }
    push_op(ctx.out, ModuleWrite::JsonSet {
        key: tenancy_key(t.id),
        value: serde_json::to_value(&t).expect("tenancy serializes"),
    });
}

/// Install a tenancy's assessment cursor and queue its leaf doc.
pub(crate) fn put_assess(ctx: &mut Ctx<'_>, id: TenancyId, state: AssessState) {
    if let Some(store) = ctx.w.module_state_mut::<HousingStore>() {
        store.assess.insert(id, state);
    }
    push_op(ctx.out, ModuleWrite::JsonSet {
        key: assess_key(id),
        value: serde_json::to_value(state).expect("assess state serializes"),
    });
}

/// Queue the allocator meta doc.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx.w.module_state::<HousingStore>().map(HousingStore::meta_doc) else {
        return;
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key: SmolStr::new_static(META_KEY), value: meta });
}

// ---------------------------------------------------------------------------
// Small world questions the whole crate asks
// ---------------------------------------------------------------------------

/// Staff: `SIM_ADMIN` or `Royalty` — the one staff predicate this crate uses, and
/// deliberately the same one `mush-economy` uses, so "staff" means one thing across
/// the two modules a landlord's officers have to reason about.
#[must_use]
pub fn is_staff(w: &World, who: ObjId) -> bool {
    mush_core::perm::has_power(w, who, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, who, Role::Royalty)
}

/// Whether an object exists and is not on its way out.
#[must_use]
pub fn alive(w: &World, id: ObjId) -> bool {
    w.get(id).is_some_and(|o| !o.flags.contains(Flags::GOING))
}

/// Whether `id` is a live [`ObjKind::Room`].
#[must_use]
pub fn is_live_room(w: &World, id: ObjId) -> bool {
    w.get(id).is_some_and(|o| o.kind == ObjKind::Room && !o.flags.contains(Flags::GOING))
}

/// Whether `id` is a character-shaped object — the subjects `add_guest` and a tenancy
/// roster admit (E5a fold, handoff 2: the library accepts any `ObjId`).
#[must_use]
pub fn is_character(w: &World, id: ObjId) -> bool {
    w.get(id)
        .is_some_and(|o| matches!(o.kind, ObjKind::Player | ObjKind::Npc) && !o.flags.contains(Flags::GOING))
}

/// The room an object is *in* — itself when it **is** a room, otherwise its
/// location. What the access hook asks of a carry's source and destination.
#[must_use]
pub fn room_of(w: &World, id: ObjId) -> Option<ObjId> {
    let o = w.get(id)?;
    match o.kind {
        ObjKind::Room => Some(id),
        _ => o.location,
    }
}

/// "Now" on a tenancy's own cadence clock — the `now` every E5a predicate and every
/// arrow takes (D-E5-14).
///
/// `None` when the clock has no record: a **missing clock never makes anything
/// happen**. Every caller treats it as "not yet" (the economy's own rule for an
/// unresolvable cadence), so a deleted cadence freezes a tenancy's ladder in place
/// rather than advancing it on a number nobody set.
#[must_use]
pub fn cadence_now(w: &World, cadence: mush_economy::CadenceId) -> Option<i64> {
    let store = w.module_state::<mush_economy::EconomyStore>()?;
    let c = store.snapshot().cadence(cadence)?;
    Some(i64::try_from(c.position).unwrap_or(i64::MAX))
}

/// The property's rooms that are still live world rooms — what the ending sweep and
/// the `Obj.home` helpers are handed.
#[must_use]
pub fn live_rooms(w: &World, p: &Property) -> BTreeSet<ObjId> {
    p.rooms.iter().copied().filter(|r| is_live_room(w, *r)).collect()
}

/// Render one object for a job note or a staff line — the same shape
/// [`render_principal`] gives an object principal.
#[must_use]
pub fn render_actor(w: &World, who: ObjId) -> String {
    render_principal(w, &PrincipalId::Object(who))
}

/// Render a principal for a housing line, without leaking anything the economy's own
/// renderer would not.
#[must_use]
pub fn render_principal(w: &World, p: &PrincipalId) -> String {
    match p {
        PrincipalId::Object(o) => w
            .get(*o)
            .map_or_else(|| format!("#{}", o.0), |x| format!("{} (#{})", x.name, o.0)),
        PrincipalId::Organization(org) => w
            .module_state::<mush_org::OrgStore>()
            .and_then(|s| s.state().org(*org).map(|o| o.name.to_string()))
            .unwrap_or_else(|| format!("org:{org}")),
        PrincipalId::System(s) => format!("sys:{s}"),
    }
}

// ---------------------------------------------------------------------------
// The module
// ---------------------------------------------------------------------------

/// The housing [`GameModule`].
///
/// Hand one to `Engine::with_modules` **after** `{org, sim, tactics, economy, jobs}`
/// (08a decision 19, 08d §2): it authorizes through org, mints title and reads
/// cadence positions through the economy, and opens its story job through jobs —
/// `on_world_loaded` runs in list order, so each of those must have booted before
/// housing's own boot reconciliation asks it anything.
#[derive(Debug, Default)]
pub struct HousingModule {
    issuer: Option<SystemPrincipalId>,
    reserved: BTreeSet<ObjId>,
}

impl HousingModule {
    /// A module with no title issuer: `@property/create` will refuse, because a
    /// property whose anchor carries no title record is a residence the economy does
    /// not know exists.
    #[must_use]
    pub fn new() -> HousingModule {
        HousingModule::default()
    }

    /// Wire the **title issuer** this server registered with
    /// [`mush_economy::EconomyModule::with_title_issuers`] (08d §2, D-E3-13).
    ///
    /// Frozen at wiring, like every other table in the E-series: a title issuer is a
    /// piece of server composition, not something a command or a document can add to,
    /// and the same principal has to be on both lists or `issue_title` refuses.
    #[must_use]
    pub fn with_issuer(mut self, issuer: SystemPrincipalId) -> HousingModule {
        self.issuer = Some(issuer);
        self
    }

    /// Wire the rooms that may **never** become part of a property (D-E5-12).
    ///
    /// The host's job, because the two that matter — `[onboarding].start_room` and
    /// `start_room_unapproved` — live in `mush-config`, which housing must not read
    /// (it is game-neutral, and the ruleset layer is above it). Everything else
    /// D-E5-12 excludes is already caught without configuration: a sim space, a domain
    /// presentation room and a common area are all reachable from outside the
    /// property, so the thoroughfare rule refuses them; Limbo is refused always; and a
    /// builder can put any room out of bounds with [`NO_PROPERTY_MARKER`].
    #[must_use]
    pub fn with_reserved_rooms(mut self, rooms: impl IntoIterator<Item = ObjId>) -> HousingModule {
        self.reserved = rooms.into_iter().collect();
        self
    }
}

impl GameModule for HousingModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(HousingStore::default());
        reg.marker_flag(NO_PROPERTY_MARKER)?;
        // ONE hook object under both points (08d §4): `may_take` *is* `may_enter` in
        // E5a, and registering one implementation under both is how that stays true
        // at the module layer too.
        let access: Arc<crate::hooks::AccessHook> = Arc::new(crate::hooks::AccessHook);
        reg.hook(mush_core::module::HookPoint::MoveAttempt, access.clone());
        reg.hook(mush_core::module::HookPoint::CarryAttempt, access);
        reg.hook(mush_core::module::HookPoint::Connected, Arc::new(crate::hooks::ReconnectNotice));
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(crate::hooks::on_obj_destroyed));
        reg.destroy_gate(Arc::new(crate::hooks::HousingDestroyGate));
        reg.job_queue(JobQueueSpec {
            name: STORY_QUEUE.into(),
            title: "Housing story windows".into(),
            min_role: Role::Wizard,
        })?;
        reg.tick(
            TICK_TAG,
            TickCadence::Coarse(std::time::Duration::from_secs(TICK_PERIOD_SECS)),
            Arc::new(crate::tick::tick),
        )?;
        crate::commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut store = crate::boot::load(&data)?;
        store.issuer = self.issuer.clone();
        store.reserved = self.reserved.clone();
        {
            let slot = ctx
                .w
                .module_state_mut::<HousingStore>()
                .ok_or_else(|| ModError::msg("housing: state slot missing"))?;
            *slot = store;
        }
        crate::boot::reconcile(ctx);
        Ok(())
    }
}

/// Whether this property is one the access hook and the destroy gate answer for.
#[must_use]
pub(crate) fn is_let(p: &Property) -> bool {
    p.status == PropertyStatus::Let
}
