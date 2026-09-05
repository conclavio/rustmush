//! The [`PartyModule`] — the cut that attaches the party record to a running world
//! (E6a, 08f §3).
//!
//! ## What lives here
//!
//! The module state slot ([`PartyStore`] — every party, the never-reused allocator,
//! and one derived index), the persistence key, the staff predicate, and the
//! registration that wires up the expiry tick ([`crate::tick`]) and the `+party`
//! surface ([`crate::commands`]).
//!
//! ## Contracts
//!
//! - **`party:{id}` is a container doc** — its state *is* the record — but a corrupt
//!   one does **not** fail the boot the way a `tenancy:{id}` does, and that is a
//!   deliberate difference of kind. A tenancy is somebody's home: booting without it
//!   means a door falls open. A party is *ephemeral by construction* — it expires
//!   after [`crate::PARTY_IDLE_TTL`] of silence — so a document that cannot be read
//!   is dropped loudly and the players make another one. The drop is leaf-shaped
//!   (`tracing::warn!` plus a `Del` fixup so the store matches RAM), never fatal.
//! - **Every document still arrives through the wire.** "Dropped, not fatal" is about
//!   the *consequence* of a bad document, never about the *check*: `Party`
//!   deserializes only through `TryFrom<PartyWire>`, so every §9 cap and every
//!   §3.1 invariant is re-checked on load (the E5a fold rule).
//! - **The allocator is floored on keys *and* meta, saturating** (the E3/E4b1 rule):
//!   a hand-written `party:18446744073709551615` parks the allocator at the ceiling,
//!   where minting refuses, rather than wrapping a live id back into use. The floor
//!   is taken from the **key** whether or not the document parses — an id that was
//!   minted was minted.
//! - **`live` is a projection**, re-derived from every record's status at boot and
//!   maintained only by [`put_party`]. It is what bounds the expiry tick's scan and
//!   [`crate::parties_of`]'s answer, and it is never authority: the status on the
//!   record is.
//! - **No party code touches money.** This module has no economy, org or faction
//!   dependency and never will (D-E6-5).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mush_core::module::{
    GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, TickCadence, WorldEventKind,
};
use mush_core::{Ctx, ModuleOp, ModuleWrite, ObjId, Outbox, Role, World};
use smol_str::SmolStr;

use crate::ids::PartyId;
use crate::record::{MAX_PARTIES, Party};

/// The module id and persistence namespace.
pub const NS: &str = "party";

/// The kv key of the allocator meta document.
pub const META_KEY: &str = "meta";

/// How often the idle-expiry tick fires (wall clock).
///
/// A latency bound and nothing more: expiry is a **pull** against each party's own
/// `expires_at`, so a fire that finds nothing due does nothing and a missed fire is
/// picked up by the next one. Sixty seconds against a seven-day TTL is four orders of
/// magnitude of slack.
pub const TICK_PERIOD_SECS: u64 = 60;

/// The expiry tick's tag.
pub const TICK_TAG: &str = "party.expire";

/// The most parties one tick fire **changes** — disbands as expired, or prunes lapsed
/// invitations on.
///
/// Not in §9 — added here so the tick is bounded like every other periodic in the
/// E-series. 32 per fire against a 60-second period drains a full
/// [`MAX_PARTIES`]-sized backlog in about two minutes, and no starvation is possible:
/// every unit of work removes its own reason to be done again (an expired party
/// leaves [`PartyStore::live_ids`]; a pruned invitation does not come back), so the
/// scan makes monotone progress from the low ids up.
pub const MAX_PARTY_EXPIRIES_PER_FIRE: usize = 32;

/// `party:{id}` — the container doc.
#[must_use]
pub fn party_key(id: PartyId) -> SmolStr {
    SmolStr::new(format!("party:{}", id.0))
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// The in-RAM party slot — the single source of truth at runtime.
#[derive(Clone, Debug, Default)]
pub struct PartyStore {
    pub(crate) parties: BTreeMap<PartyId, Party>,
    /// The ids of every party that has **not** disbanded — the expiry tick's whole
    /// working set, and the population [`MAX_PARTIES`] bounds. Derived from the
    /// status on every write and re-derived at boot, so it cannot drift into
    /// admitting a terminal party.
    pub(crate) live: BTreeSet<PartyId>,
    pub(crate) next_party: u64,
}

impl PartyStore {
    /// One party, live or terminal.
    #[must_use]
    pub fn party(&self, id: PartyId) -> Option<&Party> {
        self.parties.get(&id)
    }

    /// Every party, in id order — terminal ones included.
    pub fn parties(&self) -> impl Iterator<Item = &Party> {
        self.parties.values()
    }

    /// The ids of every party that has not disbanded, in id order.
    pub fn live_ids(&self) -> impl Iterator<Item = PartyId> + '_ {
        self.live.iter().copied()
    }

    /// Every party that has not disbanded, in id order.
    pub fn live_parties(&self) -> impl Iterator<Item = &Party> {
        self.live.iter().filter_map(|id| self.parties.get(id))
    }

    /// How many parties are live — what [`MAX_PARTIES`] bounds.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// Every live party `who` is an accepted member of, in id order, at most
    /// [`MAX_PARTIES`] of them.
    #[must_use]
    pub fn live_parties_of(&self, who: ObjId) -> Vec<PartyId> {
        self.live_parties()
            .filter(|p| p.is_member(who))
            .map(|p| p.id)
            .take(MAX_PARTIES)
            .collect()
    }

    /// Every live party `who` **leads**, in id order — what
    /// [`crate::MAX_PARTIES_PER_LEADER`] bounds and what the leader-only verbs resolve
    /// against.
    #[must_use]
    pub fn live_parties_led_by(&self, who: ObjId) -> Vec<PartyId> {
        self.live_parties()
            .filter(|p| p.is_leader(who))
            .map(|p| p.id)
            .take(MAX_PARTIES)
            .collect()
    }

    /// Every live party `who` holds an invitation to, in id order.
    #[must_use]
    pub fn live_parties_inviting(&self, who: ObjId) -> Vec<PartyId> {
        self.live_parties()
            .filter(|p| p.is_invited(who))
            .map(|p| p.id)
            .take(MAX_PARTIES)
            .collect()
    }

    /// Mint the next party id, or `None` at the ceiling (the E4b1 B2 rule: an
    /// exhausted allocator refuses one operation loudly, it never wraps a live id
    /// back into use).
    pub fn alloc_party(&mut self) -> Option<PartyId> {
        let id = PartyId(self.next_party);
        self.next_party = self.next_party.checked_add(1)?;
        Some(id)
    }

    /// The allocator resume document — a high-water mark boot never lowers.
    #[must_use]
    pub fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({ "next_party_id": self.next_party })
    }

    /// Re-derive [`PartyStore::live`] from the loaded records (boot).
    pub(crate) fn reindex_live(&mut self) {
        self.live = self
            .parties
            .values()
            .filter(|p| p.is_live())
            .map(|p| p.id)
            .collect();
    }
}

// ---------------------------------------------------------------------------
// Persist helpers
// ---------------------------------------------------------------------------

/// Queue one namespaced write.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Install a party into RAM and queue its container doc. Both always happen together
/// — there is no path that writes one without the other — and the `live` projection
/// is maintained here and nowhere else.
///
/// Panic-free on purpose: this is called from the expiry tick, where a panic takes
/// the world down, so a record that will not serialize is logged and skipped rather
/// than `expect`ed. (`Party` has no non-serializable field, so the arm is
/// unreachable; it exists because "unreachable" is a claim and `expect` is a crash.)
pub(crate) fn put_party(ctx: &mut Ctx<'_>, p: Party) {
    debug_assert!(
        p.validate().is_ok(),
        "party: a record was written that its own wire would refuse"
    );
    if let Some(store) = ctx.w.module_state_mut::<PartyStore>() {
        if p.is_terminal() {
            store.live.remove(&p.id);
        } else {
            store.live.insert(p.id);
        }
        store.parties.insert(p.id, p.clone());
    }
    match serde_json::to_value(&p) {
        Ok(value) => push_op(
            ctx.out,
            ModuleWrite::JsonSet {
                key: party_key(p.id),
                value,
            },
        ),
        Err(e) => tracing::error!(id = %p.id, error = %e, "party: record would not serialize"),
    }
}

/// Queue the allocator meta doc.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx.w.module_state::<PartyStore>().map(PartyStore::meta_doc) else {
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

/// Queue the removal of a document boot refused, so the store matches RAM.
pub(crate) fn drop_doc(ctx: &mut Ctx<'_>, key: SmolStr) {
    push_op(ctx.out, ModuleWrite::Del { key });
}

// ---------------------------------------------------------------------------
// Small world questions
// ---------------------------------------------------------------------------

/// Staff: `SIM_ADMIN` or `Royalty`.
///
/// The definition is copied from `mush-housing`'s rather than depended on: "staff"
/// has to mean one thing across the modules a player reasons about, and taking a
/// dependency on housing to say so would put a property registry inside a crate whose
/// whole claim is that it depends on `mush-core` alone (D-E6-5).
#[must_use]
pub fn is_staff(w: &World, who: ObjId) -> bool {
    mush_core::perm::has_power(w, who, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, who, Role::Royalty)
}

/// Whether `id` is a live character-shaped object — the only thing a roster or an
/// invitation admits.
#[must_use]
pub fn is_character(w: &World, id: ObjId) -> bool {
    w.get(id).is_some_and(|o| {
        matches!(o.kind, mush_core::ObjKind::Player | mush_core::ObjKind::Npc)
            && !o.flags.contains(mush_core::Flags::GOING)
    })
}

/// Render one object for a party line, leaking nothing a `look` would not.
#[must_use]
pub fn render_actor(w: &World, who: ObjId) -> String {
    w.get(who)
        .map_or_else(|| format!("{who}"), |o| format!("{} ({})", o.name, who))
}

// ---------------------------------------------------------------------------
// The module
// ---------------------------------------------------------------------------

/// The party [`GameModule`].
///
/// It has no ordering requirement against any other module: it authorizes through
/// `mush-core`'s own staff predicate, reads no clock but wall time, and asks nothing
/// of the economy — the ruleset drives [`crate::lock`] / [`crate::release`] /
/// [`crate::unlock`] from *above* both crates, which is what keeps the party from
/// being a principal (D-E6-5).
#[derive(Debug, Default)]
pub struct PartyModule;

impl PartyModule {
    /// A fresh module instance.
    #[must_use]
    pub fn new() -> PartyModule {
        PartyModule
    }
}

impl GameModule for PartyModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(PartyStore::default());
        reg.subscribe(
            WorldEventKind::ObjDestroyed,
            Arc::new(crate::hooks::on_obj_destroyed),
        );
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
        crate::boot::load(ctx, &data)
    }
}
