//! **The `PortApplier`** (doc 18 cut IC3; doc 18 §4.7 parts a–c and f, D-18-14,
//! D-18-20, D-18-27; lead rulings L1, L3–L6, L9; cut IC4's L8–L18 for part f).
//!
//! A `port` entry is applied once, under the content latch, by this applier —
//! registered from `crate::content::register_appliers` at **phase 11**, after
//! the places (5) and the routes (6) of its own pack. It does seven things and
//! nothing else:
//!
//! - **(a) the domain** — ONE passive domain per port, hosted on the lane's
//!   location room and presenting in the station's berth gallery (the room
//!   carrying `port.dock`), through `mush_sim::open_passive_domain`, which no
//!   longer counts windows (a passive domain holds none);
//! - **(b) the treasury** — the System account `system:tacsim.treasury` in
//!   `tacsim.credit`, adopted by principal (never by name) or opened;
//! - **(c) the institutions** — each a real `mush-org` org with a root scope
//!   and one scoped `tacsim.credit` account, capitalized by one-shot
//!   `create_system_obligation` tranches of at most 800,000 cr
//!   ([`crate::port::tranches`]);
//! - **(d) the goods catalog** (cut IC8) — `tacsim.goods` loaded into the market
//!   on every apply (idempotent);
//! - **(e) the vendors and their stock** (cut IC8) — the port's stock clerk, the
//!   board rooms, per vendor its storefront vault, market vendor, the clerk's
//!   `vault.withdraw` grant, its stock plan rows and its standing bids
//!   ([`crate::port::vendor`]); the stock arm of the port tick mints, dissolves and
//!   lists, never the applier;
//! - **(f) the hulls** (cut IC4) — for an entry whose `programme` names a pool,
//!   each hull laid up from the programme unit's `interior` template in the
//!   berth gallery and titled to the programme org ([`crate::port::hull`]);
//! - **(g) the site** (doc 20 cut SM.2) — for an entry with a `site` block, its
//!   sixteen block place Things and its site clerk, `DARK`, in the host room
//!   (`crate::port::site::lay_up`); the body geology tables are the definition
//!   applier's since doc 20 cut SM.5 (installed wherever a body entry is applied,
//!   and at boot; [`crate::surface::install_lane_geology`]);
//! - **(h) the rigs** (doc 20 cut SM.3a) — for an entry with a `vehicles` block,
//!   the lessor's shed and its terrain vehicles: each rig's unit, cab and hopper
//!   spaces, its hopper vault (the system principal `tacsim.vehicles`'s), its
//!   placement in the shed and its title to the lessor
//!   (`crate::port::vehicle::lay_up`). Part (f) gained step 6b the same cut:
//!   every programme hull's refit bay ([`crate::port::hull`]).
//!
//! Every act is recorded in the port map ([`crate::port::PortMap`]) written
//! **immediately after** it, in the apply's one batch — the sole dedup, since
//! the economy does not dedup obligations. The applier **submits nothing
//! authoritative and moves no money itself**: the capital settles on the
//! economy's next sweep (doc 18 §4.3).
//!
//! **Rooms resolve by `RoomGeo.key`** (lead ruling L1; the tree wins over doc 18
//! §4.5/§4.7's "through the latch"): `mush_content`'s apply takes the whole
//! latch table out of its slot while any pack applies, so a latch lookup from
//! inside an applier always fails. `RoomGeo.key` is the same `<pack>.<entry>`
//! identity, and its absence is exactly doc 18's "refuses if a room lacks
//! `RoomGeo`" ([`room_by_key`]).
//!
//! **Update never re-capitalizes, renames or re-hosts** (lead ruling L4): those
//! differences are refused as drift by [`EntryApplier::check`], before any
//! write. Each org row records the capital it was created for, so a changed
//! base capital is refused even when its tranches merely extend the recorded ones
//! (`capital_drift`; D-18-27, L4). **A raise is authored as its own appended
//! tranches** (doc 20 §7.3, cut SM.5: `PortOrgDef.raises`, minted by
//! [`crate::port::def::capital_schedule`]), and the row records each raise once
//! its tranches are begun; a raise edited, dropped or reordered is drift. A new
//! org in a later version is created and
//! capitalized; a changed `faction` word is accepted and writes nothing; a map
//! row whose org, account or obligation is gone from its owner's store is
//! refused, never re-created. Hulls re-apply by map row, never by name: a grown
//! pool makes only the new hulls, and a shrunk pool, a changed prefix or a
//! recorded object gone is refused as drift (cut IC4, L10).
//! Vendors re-apply by map row too (cut IC8): a recorded stock row is never
//! re-issued or re-sized (Q-18-19, lead ruling L6), a recorded bid is revised in
//! place, and a re-priced ask's Draw listing is withdrawn for the arm to relist.
//! Retire withdraws the vendors' Draw listings and standing bids (cut IC8) and
//! leaves everything else standing, the hulls included; a vendor with an open
//! deal refuses it.

use std::collections::BTreeMap;

use mush_atlas::RecordKey;
use mush_content::{EntryApplier, Pack, RawEntry};
use mush_core::{CanonicalId, Ctx, ObjId, ObjKind, World};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, CreditPolicy, EconomicClock, EconomicSchedule,
    EconomyStore, GracePolicy, InstrumentId, ObligationCause, ObligationId, ObligationSpec,
};
use mush_org::{
    OrgAction, OrgId, OrgMutationOp, OrgMutationPlan, OrgScope, OrgScopeId, OrgStatus, OrgStore,
    Organization, PrincipalId, RulesValue, ScopeStatus, SystemPrincipalId,
};
use smol_str::SmolStr;

use crate::economy::{CREDIT, CREDIT_SCALE, GameEconomyConfig};
use crate::pack::PackEntry;
use crate::port::def::{PortDef, capital_schedule, room_key};
use crate::port::map::{PortMap, PortMapState, PortOrgRow, TrancheRow};
use crate::store::{TacsimStore, put_port_map};

/// The applier of the `port` kind (doc 18 §4.7).
pub struct PortApplier;

/// The kind word.
const PORT_KIND: &str = "port";

/// **The live room an atlas entry made** (lead ruling L1): the room whose
/// `RoomGeo.key` is `key`, live, a room, not `GOING`. `None` when no `RoomGeo`
/// carries the key — the room is unresolved.
#[must_use]
pub fn room_by_key(w: &World, key: &RecordKey) -> Option<ObjId> {
    let atlas = w.module_state::<mush_atlas::AtlasState>()?;
    atlas
        .records
        .roomgeo
        .values()
        .filter(|geo| &geo.key == key)
        .map(|geo| geo.room)
        .find(|room| {
            w.get(*room).is_some_and(|o| {
                o.kind == ObjKind::Room && !o.flags.contains(mush_core::Flags::GOING)
            })
        })
}

/// **Doc 18 cut IC8's vault allocator reach** (the state-slot gate): the economy's
/// allocator, reached only here so `apply.rs`'s rows cover `crate::port::vendor`.
///
/// # Errors
///
/// When the economy is not composed or the allocator is spent.
pub(crate) fn alloc_vault_id(ctx: &mut Ctx<'_>) -> Result<mush_economy::VaultId, String> {
    ctx.w
        .module_state_mut::<EconomyStore>()
        .ok_or_else(|| "the economy is not composed".to_owned())?
        .alloc_vault()
        .ok_or_else(|| "the vault allocator is spent".to_owned())
}

/// **Doc 18 cut IC8's grant allocator reach** (the state-slot gate): the org store's
/// allocator, reached only here so `apply.rs`'s rows cover `crate::port::vendor`.
///
/// # Errors
///
/// When organizations are not composed.
pub(super) fn alloc_grant_id(ctx: &mut Ctx<'_>) -> Result<mush_org::GrantId, String> {
    ctx.w
        .module_state_mut::<OrgStore>()
        .map(OrgStore::alloc_grant)
        .ok_or_else(|| "organizations are not composed".to_owned())
}

/// The `tacsim.credit` instrument id.
fn credit() -> Result<InstrumentId, String> {
    InstrumentId::new(CREDIT).ok_or_else(|| format!("{CREDIT} is not an instrument id"))
}

/// The credit's minor-unit scale on this world: the registered instrument's, or
/// the game's own [`CREDIT_SCALE`] before it is registered.
pub(crate) fn credit_scale(w: &World) -> u8 {
    w.module_state::<EconomyStore>()
        .and_then(|store| {
            InstrumentId::new(CREDIT)
                .and_then(|id| store.snapshot().instrument(&id).map(|def| def.scale))
        })
        .unwrap_or(CREDIT_SCALE)
}

/// `major` in the credit's minor units at `scale`, checked.
pub(crate) fn minor_of(major: u64, scale: u8) -> Option<i64> {
    let factor = 10_i64.checked_pow(u32::from(scale))?;
    i64::try_from(major).ok()?.checked_mul(factor)
}

/// Every room reference of a port entry, in report order: the gallery, each
/// role's room, the programme's office.
pub(crate) fn room_references(def: &PortDef) -> Vec<&str> {
    let mut refs: Vec<&str> = vec![def.gallery.as_str()];
    refs.extend(def.rooms.values().map(String::as_str));
    if let Some(programme) = &def.programme {
        refs.push(programme.office.as_str());
    }
    refs
}

/// The lowest-id Active account `owner` holds in `instrument`, unscoped or not.
fn lowest_account(
    store: &EconomyStore,
    owner: &PrincipalId,
    instrument: &InstrumentId,
) -> Option<AccountId> {
    store
        .snapshot()
        .accounts_of(owner)
        .filter(|a| a.status == AccountStatus::Active && &a.instrument == instrument)
        .map(|a| a.id)
        .min()
}

/// What [`PortApplier::drift`] names in its refusals and compares against.
#[derive(Clone, Copy)]
struct DriftScope<'a> {
    /// The port entry.
    id: &'a CanonicalId,
    /// Its map's document key.
    key: &'a SmolStr,
    /// The authored entry.
    def: &'a PortDef,
    /// The resolved gallery.
    gallery: ObjId,
    /// The credit's minor-unit scale.
    scale: u8,
}

/// How a recorded org row's capital disagrees with the pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapitalDrift {
    /// The authored capital is not the one the row was created for: a port
    /// update never re-capitalizes (L4, D-18-27), even when the new tranche
    /// list extends the recorded one.
    Changed {
        /// The capital the row records, major units.
        recorded: u64,
    },
    /// The recorded tranche rows are not a prefix of the tranches of the
    /// row's own capital (an index out of range, repeated, or an amount that
    /// is not that tranche's): a hand-edited or damaged map, never re-issued.
    Inconsistent {
        /// The capital the row records, major units.
        recorded: u64,
    },
    /// The row's recorded raises are not a prefix of the authored ones: a raise
    /// edited, dropped or reordered (doc 20 §7.3, cut SM.5). Never re-issued.
    Raises,
}

/// **The capital rule of one present map row** (L4, D-18-27; doc 18 §4.7
/// "Update never re-capitalizes"; doc 20 §7.3, cut SM.5). Checked in this order:
///
/// 1. the row's recorded base `capital_major` must equal the authored one — a
///    changed base is refused even when the new tranches merely extend the
///    recorded ones (4,000,000 → 4,800,000 adds a sixth 800,000 beside five that
///    match): [`CapitalDrift::Changed`];
/// 2. the row's recorded raises must be a prefix of the authored ones — capital
///    is added only by appending a raise, never by editing, dropping or
///    reordering one: [`CapitalDrift::Raises`];
/// 3. the recorded tranche rows must be distinct indices of
///    `capital_schedule(capital_major, authored.raises)` with that tranche's
///    amount: [`CapitalDrift::Inconsistent`].
///
/// The indices not yet recorded are what an apply that stopped part way (or a
/// raise not yet begun) left to make, and the next pass makes them. `None` is
/// no drift.
pub(crate) fn capital_drift(
    authored: &crate::port::def::PortOrgDef,
    row: &PortOrgRow,
    scale: u8,
) -> Option<CapitalDrift> {
    let recorded = row.capital_major;
    if recorded != authored.capital_major {
        return Some(CapitalDrift::Changed { recorded });
    }
    if !authored.raises.starts_with(&row.raises) {
        return Some(CapitalDrift::Raises);
    }
    let want = capital_schedule(recorded, &authored.raises);
    let mut seen = std::collections::BTreeSet::new();
    let consistent = row.capital.iter().all(|t| {
        seen.insert(t.n)
            && usize::try_from(t.n)
                .ok()
                .and_then(|n| want.get(n))
                .and_then(|major| minor_of(*major, scale))
                == Some(t.amount_minor)
    });
    (!consistent).then_some(CapitalDrift::Inconsistent { recorded })
}

impl PortApplier {
    /// **Every refusal that can be decided before anything mutates** (doc 18
    /// §4.7; L1, L4): the layers, the map's readability, the rooms and the
    /// gallery's marker, the mint cap against the largest tranche still to be
    /// made, every drift of the authored entry against a present map, and (cut
    /// IC4) every hull refusal of [`crate::port::hull::preflight`].
    #[allow(clippy::too_many_lines)]
    fn preflight(
        reg: &mush_core::Registry,
        w: &World,
        pack: &CanonicalId,
        id: &CanonicalId,
        def: &PortDef,
    ) -> Result<(), String> {
        let Some(tacsim) = w.module_state::<TacsimStore>() else {
            return Err(format!(
                "{id}: the TacSim layer is not composed on this world"
            ));
        };
        let Some(economy) = w.module_state::<EconomyStore>() else {
            return Err(format!("{id}: the economy is not composed on this world"));
        };
        let Some(orgs) = w.module_state::<OrgStore>() else {
            return Err(format!(
                "{id}: organizations are not composed on this world"
            ));
        };
        let Some(spatial) = w.module_state::<mush_sim::data::SpatialState>() else {
            return Err(format!(
                "{id}: the simulation is not composed on this world"
            ));
        };
        let key = crate::port::port_map_key(pack, id);
        let map = match tacsim.port_map(pack, id) {
            PortMapState::Unreadable => {
                let sentence = format!(
                    "{id}: the port map {key} is unreadable; it is never re-created — repair or \
                     delete the document by hand"
                );
                tracing::error!("{sentence}");
                return Err(sentence);
            }
            PortMapState::Absent => None,
            PortMapState::Present(map) => Some(map),
        };
        // The rooms (L1): every reference resolves by its `RoomGeo.key`.
        for reference in room_references(def) {
            let record =
                room_key(&def.area, reference).map_err(|detail| format!("{id}: {detail}"))?;
            if room_by_key(w, &record).is_none() {
                return Err(format!(
                    "{id}: the room {reference} is not on this world (no room carries its RoomGeo)"
                ));
            }
        }
        let gallery_key =
            room_key(&def.area, &def.gallery).map_err(|detail| format!("{id}: {detail}"))?;
        let gallery = room_by_key(w, &gallery_key)
            .ok_or_else(|| format!("{id}: the gallery {} is not on this world", def.gallery))?;
        if !w.has_marker(gallery, mush_atlas::def::PORT_DOCK_MARKER) {
            return Err(format!(
                "{id}: the gallery {} does not carry {}",
                def.gallery,
                mush_atlas::def::PORT_DOCK_MARKER
            ));
        }
        let scale = credit_scale(w);
        // Drift against a present map (L4) first, so a changed capital is
        // refused as the drift it is, never as the cap or the freeze it would
        // run into.
        if let Some(map) = map {
            PortApplier::drift(
                economy,
                orgs,
                spatial,
                &DriftScope {
                    id,
                    key: &key,
                    def,
                    gallery,
                    scale,
                },
                map,
            )?;
        }
        // The vendors (cut IC8, parts d–e): the market, the catalogue rows, the
        // storefronts, signatories, clerk and board rooms, and every recorded
        // stock-row and bid drift.
        crate::port::vendor::preflight(w, id, def, map)?;
        // Doc 20 cut SM.2 (part g): every recorded block place and the site clerk
        // is live; a recorded Thing gone is never re-created.
        crate::port::site::preflight(w, id, map)?;
        // The hulls (cut IC4, part f): the template, the issuer, the exit words
        // and every hull drift, and how many hulls this pass would title.
        let hulls_to_title = crate::port::hull::preflight(reg, w, pack, id, def, map)?;
        // Doc 20 cut SM.3a (part h): the rig pool's template, issuer and every rig
        // drift, and how many rigs this pass would title.
        let rigs_to_title = crate::port::vehicle::preflight(w, pack, id, def, map)?;
        // The mint cap (D-18-14): the largest tranche still to be created. The
        // drift rule above makes every recorded row's capital the authored one,
        // so what is still to be created is exactly the unrecorded indices.
        let recorded = |org: &CanonicalId| -> Vec<&TrancheRow> {
            map.and_then(|m| m.orgs.get(org))
                .map(|row| row.capital.iter().collect())
                .unwrap_or_default()
        };
        let mut largest: u64 = 0;
        let mut to_make: usize = 0;
        for org in &def.orgs {
            let made = recorded(&org.id);
            // Doc 20 cut SM.5: the base's tranches, then each raise's, appended.
            for (n, major) in capital_schedule(org.capital_major, &org.raises)
                .into_iter()
                .enumerate()
            {
                let n = u32::try_from(n).unwrap_or(u32::MAX);
                if !made.iter().any(|row| row.n == n) {
                    largest = largest.max(major);
                    to_make = to_make.saturating_add(1);
                }
            }
        }
        let cap = economy.config().cap_major(&credit()?);
        if largest > cap {
            return Err(format!(
                "{id}: a capital tranche is {largest} cr but this world's mint cap for {CREDIT} \
                 is {cap} cr; set [economy.instruments.\"{CREDIT}\"].mint_obligation_cap_major to \
                 at least {largest}"
            ));
        }
        // A frozen economy (its boot integrity checks failed) refuses every
        // obligation (`create_system_obligation` answers `EconomyFrozen`) but
        // not an account opening, so an apply that went ahead would open the
        // domain, the treasury, an org and its account and then stop at the
        // first tranche — the "mutates, then refuses" the check phase exists to
        // prevent. Every org a pass still has to make has a tranche to make
        // (capital is at least 1), so "a tranche is still to be created" is
        // exactly "this pass writes to the economy". A hull title is an economy
        // write too (cut IC4): `issue_title` refuses on a frozen economy, after
        // the hull's objects exist. A rig title is one too (doc 20 cut SM.3a).
        if (to_make > 0 || hulls_to_title > 0 || rigs_to_title > 0) && economy.is_frozen() {
            if hulls_to_title == 0 && rigs_to_title == 0 {
                return Err(format!(
                    "{id}: the economy is frozen on this world, so {to_make} capital tranche(s) \
                     cannot be issued; nothing is applied while it is frozen"
                ));
            }
            if rigs_to_title == 0 {
                return Err(format!(
                    "{id}: the economy is frozen on this world, so {to_make} capital tranche(s) \
                     and {hulls_to_title} hull title(s) cannot be issued; nothing is applied \
                     while it is frozen"
                ));
            }
            return Err(format!(
                "{id}: the economy is frozen on this world, so {to_make} capital tranche(s) and \
                 {hulls_to_title} hull title(s) cannot be issued, nor {rigs_to_title} rig \
                 title(s); nothing is applied while it is frozen"
            ));
        }
        Ok(())
    }

    /// **Every drift of the authored entry against its present map** (L4; doc
    /// 18 §4.7 "never re-capitalizes or re-issues"): the presentation, the
    /// domain, the treasury, and per recorded org its existence, account,
    /// obligations, name and capital. Each refusal names the map key; nothing
    /// that is gone is ever re-created.
    #[allow(clippy::too_many_lines)]
    fn drift(
        economy: &EconomyStore,
        orgs: &OrgStore,
        spatial: &mush_sim::data::SpatialState,
        scope: &DriftScope<'_>,
        map: &PortMap,
    ) -> Result<(), String> {
        let DriftScope {
            id,
            key,
            def,
            gallery,
            scale,
        } = *scope;
        if let Some(presentation) = map.presentation
            && presentation != gallery.0
        {
            return Err(format!(
                "{id}: the port map {key} presents in #{presentation}, but the gallery {} is #{}; \
                 a port update never re-hosts its domain",
                def.gallery, gallery.0
            ));
        }
        if let Some(domain) = map.domain {
            let live = spatial
                .domains
                .get(&mush_sim::data::DomainId(domain))
                .filter(|d| d.status == mush_sim::data::DomainStatus::Active);
            let Some(record) = live else {
                return Err(format!(
                    "{id}: the port map {key} names domain #{domain}, which is not active on this \
                     world; it is never re-created — repair the map by hand"
                ));
            };
            if map.host.is_some_and(|host| host != record.host.0)
                || record.presentation_location != gallery
            {
                return Err(format!(
                    "{id}: domain #{domain} in the port map {key} is hosted or presented \
                     elsewhere; a port update never re-hosts its domain"
                ));
            }
        }
        if let Some(account) = map.treasury {
            let owner = SystemPrincipalId::new(&def.treasury)
                .map(PrincipalId::System)
                .ok_or_else(|| {
                    format!(
                        "{id}: treasury `{}` is not a system principal",
                        def.treasury
                    )
                })?;
            let ok = economy
                .snapshot()
                .account(AccountId(account))
                .is_some_and(|a| {
                    a.status == AccountStatus::Active
                        && a.owner == owner
                        && a.instrument.as_str() == CREDIT
                });
            if !ok {
                return Err(format!(
                    "{id}: the port map {key} names treasury account #{account}, which is not \
                     {}'s active {CREDIT} account; it is never re-created — repair the map by hand",
                    def.treasury
                ));
            }
        }
        for (org_id, row) in &map.orgs {
            let Some(authored) = def.orgs.iter().find(|org| &org.id == org_id) else {
                return Err(format!(
                    "{id}: org {org_id} is in the port map {key} but no longer authored; orgs are \
                     never retired by a port update"
                ));
            };
            let Some(live) = orgs.state().org(OrgId(row.org)) else {
                return Err(format!(
                    "{id}: org {org_id} in the port map {key} names org #{}, which does not \
                     exist; it is never re-created — repair the map by hand",
                    row.org
                ));
            };
            if let Some(account) = row.account
                && economy.snapshot().account(AccountId(account)).is_none()
            {
                return Err(format!(
                    "{id}: org {org_id} in the port map {key} names account #{account}, which \
                     does not exist; it is never re-created — repair the map by hand"
                ));
            }
            if let Some(gone) = row.capital.iter().find(|t| {
                economy
                    .snapshot()
                    .obligation(ObligationId(t.obligation))
                    .is_none()
            }) {
                return Err(format!(
                    "{id}: org {org_id} in the port map {key} names obligation #{}, which does \
                     not exist; it is never re-created — repair the map by hand",
                    gone.obligation
                ));
            }
            if live.name.as_str() != authored.name {
                return Err(format!(
                    "{id}: org {org_id} is named {} on this world but {} in the pack; this server \
                     cannot rename an org",
                    live.name, authored.name
                ));
            }
            match capital_drift(authored, row, scale) {
                None => {}
                // Doc 20 cut SM.5 (§7.3): a raise is appended, never edited.
                Some(CapitalDrift::Raises) => {
                    return Err(format!(
                        "{id}: org {org_id}'s raises {:?} do not extend the {:?} already issued; a \
                         port update never re-capitalizes",
                        authored.raises, row.raises
                    ));
                }
                Some(CapitalDrift::Changed { recorded }) => {
                    return Err(format!(
                        "{id}: org {org_id} capital {} cr differs from the {recorded} cr already \
                         issued; a port update never re-capitalizes",
                        authored.capital_major
                    ));
                }
                Some(CapitalDrift::Inconsistent { recorded }) => {
                    return Err(format!(
                        "{id}: org {org_id}'s tranche rows in the port map {key} are not the \
                         tranches of its {recorded} cr; they are never re-issued — repair the map \
                         by hand"
                    ));
                }
            }
        }
        Ok(())
    }

    /// The authored entry, decoded and checked on its own (the pack-wide rules
    /// ran in [`EntryApplier::check`]).
    fn decode(id: &CanonicalId, entry: &RawEntry) -> Result<PortDef, String> {
        let def: PortDef = entry.decode().map_err(|e| format!("{id}: {e}"))?;
        def.check()
            .map_err(|detail| format!("port entry {id}: {detail}"))?;
        Ok(def)
    }

    /// **The apply** (create and update alike): parts a–f in order, the map
    /// written after every act; nothing is submitted. It answers `Ok(None)`: the hulls are
    /// recorded in the port map, not in the latch (the host room is the
    /// `location` entry's).
    #[allow(clippy::too_many_lines)]
    fn put(
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        let pack_id = pack.manifest().id().clone();
        let def = PortApplier::decode(id, entry)?;
        // The same answers `check` gave before the apply touched anything; asked
        // again so a refusal here can never follow a partial write of this entry.
        PortApplier::preflight(ctx.reg, ctx.w, &pack_id, id, &def)?;
        // (0) The host, the gallery, the map.
        let host = placed
            .get(&def.location)
            .copied()
            .filter(|room| {
                ctx.w.get(*room).is_some_and(|o| {
                    o.kind == ObjKind::Room && !o.flags.contains(mush_core::Flags::GOING)
                })
            })
            .ok_or_else(|| format!("{id}: the pack placed no live room for {}", def.location))?;
        let gallery_key =
            room_key(&def.area, &def.gallery).map_err(|detail| format!("{id}: {detail}"))?;
        let gallery = room_by_key(ctx.w, &gallery_key)
            .ok_or_else(|| format!("{id}: the gallery {} is not on this world", def.gallery))?;
        let mut map = match ctx
            .w
            .module_state::<TacsimStore>()
            .map(|store| store.port_map(&pack_id, id))
        {
            Some(PortMapState::Present(map)) => map.clone(),
            Some(PortMapState::Absent) => PortMap::new(pack_id.clone(), id.clone()),
            Some(PortMapState::Unreadable) | None => {
                return Err(format!(
                    "{id}: the port map {} cannot be read",
                    crate::port::port_map_key(&pack_id, id)
                ));
            }
        };
        let key = map.key();
        if map.host.is_some_and(|recorded| recorded != host.0) {
            return Err(format!(
                "{id}: the port map {key} names host #{}, but the lane placed {} at #{}; a port \
                 update never re-hosts its domain",
                map.host.unwrap_or_default(),
                def.location,
                host.0
            ));
        }

        // (a) The domain: one passive domain, hosted on the place room and
        // presenting in the gallery. Never adopted by host or by name.
        if map.domain.is_none() {
            let standing = ctx
                .w
                .module_state::<mush_sim::data::SpatialState>()
                .and_then(|state| state.domain_of_host(host))
                .map(|record| record.id);
            if let Some(domain) = standing {
                return Err(format!(
                    "{id}: {} already hosts domain #{domain}, which this port's map does not name",
                    mush_core::output::display_name(ctx.w, host)
                ));
            }
            let domain = mush_sim::open_passive_domain(ctx, host, gallery)
                .map_err(|e| format!("{id}: {e}"))?;
            map.domain = Some(domain.0);
            map.host = Some(host.0);
            map.presentation = Some(gallery.0);
            put_port_map(ctx, &map);
        }

        // (g) Doc 20 cut SM.2 (§3.1, §3.4): a site's sixteen block place Things and its
        // site clerk, in the host room, recorded. Nothing is submitted. Parts (c)–(f)
        // are no-ops for a site (no orgs, vendors or programme); (b) runs as for every
        // port. The body tables are no longer installed here (doc 20 cut SM.5, lead
        // ruling 9): the definition applier installs a body's table wherever its entry
        // is applied, and the boot installs every table.
        if let Some(site) = &def.site {
            crate::port::site::lay_up(ctx, id, site, &mut map, host)?;
        }

        // (b) The treasury (L9: the credit first, create-only — a roster with no
        // sheet module has none).
        let instrument = crate::economy::ensure_instrument(ctx, &GameEconomyConfig::default())
            .ok_or_else(|| format!("{id}: {CREDIT} could not be registered"))?;
        // Doc 20 cut SM.5 (§9.1): an entry that authors the licence block registers the
        // licence instrument the same create-only way (a roster with no sheet module has
        // none); a refusal is logged, and the readiness Licence line names it.
        if def.licence.is_some() {
            let _ = crate::economy::ensure_licence_instrument(ctx);
        }
        let faucet = SystemPrincipalId::new(&def.treasury).ok_or_else(|| {
            format!(
                "{id}: treasury `{}` is not a system principal",
                def.treasury
            )
        })?;
        let treasury = match map.treasury {
            Some(account) => AccountId(account),
            None => {
                let owner = PrincipalId::System(faucet.clone());
                let adopted = ctx
                    .w
                    .module_state::<EconomyStore>()
                    .and_then(|store| lowest_account(store, &owner, &instrument));
                let account = match adopted {
                    Some(account) => account,
                    None => {
                        let account = ctx
                            .w
                            .module_state_mut::<EconomyStore>()
                            .ok_or_else(|| format!("{id}: the economy is not composed"))?
                            .alloc_account()
                            .ok_or_else(|| format!("{id}: the account allocator is spent"))?;
                        mush_economy::open_account(
                            ctx,
                            Account {
                                id: account,
                                owner,
                                instrument: instrument.clone(),
                                scope: None,
                                name: SmolStr::new(&def.treasury),
                                status: AccountStatus::Active,
                                posted_balance: Amount::ZERO,
                                held_amount: Amount::ZERO,
                                last_hold_seq: 0,
                                credit: CreditPolicy::unbounded(),
                                last_journal_seq: 0,
                                revision: 1,
                                approval: None,
                            },
                        );
                        account
                    }
                };
                map.treasury = Some(account.0);
                put_port_map(ctx, &map);
                account
            }
        };

        // (c) The institutions, in authored order.
        let scale = credit_scale(ctx.w);
        for org in &def.orgs {
            // The org and its root scope, in ONE `CreateOrg` op, exactly as
            // `@faction/org` makes one (`mush-faction/src/econ.rs`).
            if !map.orgs.contains_key(&org.id) {
                let (org_id, root_id) = {
                    let store = ctx
                        .w
                        .module_state_mut::<OrgStore>()
                        .ok_or_else(|| format!("{id}: organizations are not composed"))?;
                    (store.alloc_org(), store.alloc_scope())
                };
                let record = Organization {
                    id: org_id,
                    name: SmolStr::new(&org.name),
                    aliases: Vec::new(),
                    status: OrgStatus::Active,
                    hidden: false,
                    charter: String::new(),
                    root_scope: root_id,
                    revision: 1,
                    ruleset_ext: RulesValue::default(),
                };
                let root = OrgScope {
                    id: root_id,
                    org: org_id,
                    parent: None,
                    name: SmolStr::new_static("root"),
                    status: ScopeStatus::Active,
                    depth: 0,
                    created_rev: 1,
                    revision: 1,
                };
                let plan =
                    OrgMutationPlan::of(vec![OrgMutationOp::CreateOrg { org: record, root }]);
                mush_org::submit(
                    ctx,
                    &plan,
                    OrgAction {
                        action: SmolStr::new_static("org.create"),
                        org: Some(org_id),
                        detail: serde_json::json!({
                            "port": format!("{pack_id}/{id}"),
                            "org": org.id.to_string(),
                            "name": org.name,
                        }),
                    },
                )
                .map_err(|e| format!("{id}: org {}: {e}", org.id))?;
                map.orgs.insert(
                    org.id.clone(),
                    PortOrgRow {
                        org: org_id.0,
                        root_scope: root_id.0,
                        capital_major: org.capital_major,
                        account: None,
                        capital: Vec::new(),
                        raises: Vec::new(),
                    },
                );
                put_port_map(ctx, &map);
            }
            let Some(row) = map.orgs.get(&org.id).cloned() else {
                return Err(format!("{id}: org {} has no map row", org.id));
            };
            // Its scoped account, `open_root_account`'s shape.
            let account = match row.account {
                Some(account) => AccountId(account),
                None => {
                    let account = ctx
                        .w
                        .module_state_mut::<EconomyStore>()
                        .ok_or_else(|| format!("{id}: the economy is not composed"))?
                        .alloc_account()
                        .ok_or_else(|| format!("{id}: the account allocator is spent"))?;
                    mush_economy::open_account(
                        ctx,
                        Account {
                            id: account,
                            owner: PrincipalId::Organization(OrgId(row.org)),
                            instrument: instrument.clone(),
                            scope: Some(OrgScopeId(row.root_scope)),
                            name: SmolStr::new(format!("{}/root", org.name)),
                            status: AccountStatus::Active,
                            posted_balance: Amount::ZERO,
                            held_amount: Amount::ZERO,
                            last_hold_seq: 0,
                            credit: CreditPolicy::forbid(),
                            last_journal_seq: 0,
                            revision: 1,
                            approval: None,
                        },
                    );
                    if let Some(row) = map.orgs.get_mut(&org.id) {
                        row.account = Some(account.0);
                    }
                    put_port_map(ctx, &map);
                    account
                }
            };
            // Its capital: one one-shot faucet obligation per tranche not yet
            // made. A recorded tranche is skipped, so a re-apply posts nothing.
            // Doc 20 cut SM.5 (§7.3): the schedule is the base's tranches and then
            // each authored raise's, appended, so a raise on a kept world makes
            // exactly its own new tranches.
            for (n, major) in capital_schedule(org.capital_major, &org.raises)
                .into_iter()
                .enumerate()
            {
                let n = u32::try_from(n).map_err(|_| format!("{id}: too many tranches"))?;
                let made = map
                    .orgs
                    .get(&org.id)
                    .is_some_and(|row| row.capital.iter().any(|t| t.n == n));
                if made {
                    continue;
                }
                let amount_minor = minor_of(major, scale)
                    .ok_or_else(|| format!("{id}: org {}: a tranche overflows", org.id))?;
                let obligation = mush_economy::create_system_obligation(
                    ctx,
                    &faucet,
                    ObligationSpec {
                        debit_account: treasury,
                        credit_account: account,
                        amount: Amount::from_minor(amount_minor),
                        schedule: EconomicSchedule::new(
                            EconomicClock::RealUtc,
                            ctx.w.now(),
                            None,
                            1,
                        ),
                        grace: GracePolicy::default(),
                        notify_on_success: false,
                        cause: ObligationCause::Ruleset {
                            namespace: SmolStr::new_static("tacsim.port"),
                        },
                    },
                )
                .map_err(|e| format!("{id}: org {} tranche {n}: {e}", org.id))?;
                if let Some(row) = map.orgs.get_mut(&org.id) {
                    row.capital.push(TrancheRow {
                        n,
                        obligation: obligation.0,
                        amount_minor,
                    });
                }
                put_port_map(ctx, &map);
            }
            // Doc 20 cut SM.5: the row records each raise once its tranches are
            // begun (here, all made), so a later version's raises are compared
            // against it; an apply that stopped part way completes on the next pass.
            if let Some(row) = map.orgs.get_mut(&org.id)
                && row.raises != org.raises
            {
                row.raises.clone_from(&org.raises);
                put_port_map(ctx, &map);
            }
        }

        // (d)–(e) The goods catalog, then the vendors and their stock (cut IC8):
        // after the institutions, because each vendor's vault and account are its
        // org's. Normal-lane writes only; the stock arm mints and lists.
        crate::port::vendor::put(ctx, id, &def, &mut map)?;

        // (f) The hulls (cut IC4; L8): after the institutions, because the
        // title goes to the programme org part c made in this same pass.
        if let Some(programme) = &def.programme {
            let domain = map
                .domain
                .map(mush_sim::data::DomainId)
                .ok_or_else(|| format!("{id}: no domain is recorded"))?;
            crate::port::hull::lay_up(ctx, id, programme, &mut map, domain, gallery)?;
        }

        // (h) Doc 20 cut SM.3a: the lessor's rigs, after the hulls (and the
        // institutions, whose org the rigs are titled to).
        if let Some(vehicles) = &def.vehicles {
            crate::port::vehicle::lay_up(ctx, id, vehicles, &mut map)?;
        }
        Ok(None)
    }
}

/// **The leased-hull retire refusal's text** (doc 18 §4.7; cut IC6).
#[must_use]
pub(crate) fn leased_refusal(id: &CanonicalId, mark: &str) -> String {
    format!(
        "{id}: hull {mark} is chartered to a pilot; a port with a leased hull cannot be retired"
    )
}

/// **Whether `(pack, id)` may retire** (doc 18 §4.7 "retire_check refuses while
/// a hull is leased"; cut IC6): refused when any hull row of its port map
/// carries a live lease from one of the map's institutions (the programme's
/// Trust is one of them). An absent or unreadable map records no hull, so it
/// refuses nothing here.
fn retire_refusal(w: &World, pack: &CanonicalId, id: &CanonicalId) -> Result<(), String> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Ok(());
    };
    let PortMapState::Present(map) = store.port_map(pack, id) else {
        return Ok(());
    };
    let lessors: Vec<PrincipalId> = map
        .orgs
        .values()
        .map(|row| PrincipalId::Organization(OrgId(row.org)))
        .collect();
    for row in &map.hulls {
        let hull = ObjId(row.hull);
        if lessors
            .iter()
            .any(|lessor| crate::port::coop::trust_lease_on(w, lessor, hull).is_some())
        {
            return Err(leased_refusal(id, &row.mark));
        }
    }
    // Doc 18 cut IC8: a vendor with an open market deal keeps the port standing.
    crate::port::vendor::retire_refusal(w, id, map)
}

impl EntryApplier for PortApplier {
    fn kinds(&self) -> Vec<CanonicalId> {
        CanonicalId::new(PORT_KIND).into_iter().collect()
    }

    fn phase(&self) -> u8 {
        // After the places (5) and the routes (6) of its own pack: the host is
        // the `location` entry's room (doc 18 §4.7).
        11
    }

    /// **Every refusal that does not need a write** (doc 18 §4.7): the whole
    /// pack's rules as `crate::pack::rules_pack_of` runs them on the pack the
    /// latch is applying (a refusal names the entry), then this entry's
    /// preflight — the map, the rooms, the mint cap and every drift. A refusal
    /// here refuses the whole pack and writes nothing.
    fn check(
        &self,
        ctx: &Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        _entry: &RawEntry,
    ) -> Result<(), String> {
        let rules = crate::pack::rules_pack_of(pack.clone()).map_err(|e| e.to_string())?;
        let Some(PackEntry::Port(def)) = rules.entry(id) else {
            return Err(format!("{id}: not a port entry"));
        };
        PortApplier::preflight(ctx.reg, ctx.w, pack.manifest().id(), id, def)
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        PortApplier::put(ctx, pack, id, entry, placed)
    }

    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        _existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        // In place: every map row is reused, and what was made stands.
        PortApplier::put(ctx, pack, id, entry, placed)
    }

    fn retire_check(
        &self,
        ctx: &Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        _existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        // Hulls stand on retire (doc 18 §4.7). Doc 18 §4.7 "retire_check
        // refuses while a hull is leased" (cut IC6, deferred here by IC4): a
        // port with a chartered hull is not retired out from under its pilot.
        // The lessor is read from the map's own org rows — every institution
        // the port made, the programme's Trust among them — so the check needs
        // no pack version to name the programme org. Cut IC8: a vendor with an
        // open market deal (a contract not yet settled) refuses too.
        retire_refusal(ctx.w, pack, id)
    }

    fn retire(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        _existing: Option<ObjId>,
    ) -> Result<(), String> {
        // Doc 18 §4.7: "Retire withdraws asks and bids; matter, receipts, orgs,
        // accounts and hulls stand." Cut IC8: the vendors' Draw listings and
        // standing bids are withdrawn (neither stages anything); any other
        // vendor listing is left and logged. The map is kept so a later apply
        // reuses every row — except the bid rows this retire withdrew, which
        // it forgets so that apply posts them afresh (never "intake taken").
        let map =
            ctx.w
                .module_state::<TacsimStore>()
                .and_then(|store| match store.port_map(pack, id) {
                    PortMapState::Present(map) => Some(map.clone()),
                    PortMapState::Absent | PortMapState::Unreadable => None,
                });
        if let Some(mut map) = map {
            crate::port::vendor::retire_listings(ctx, id, &mut map);
        }
        tracing::info!(
            %pack, %id,
            "tacsim: a port entry retired; its vendors' asks and bids are withdrawn, and its \
             domain, orgs, accounts, obligations, vaults, vendors, stock, clerk and hulls stand \
             and its port map is kept"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row created for `capital_major` with the first `made` of its
    /// tranches recorded (obligation ids from 100).
    fn row(capital_major: u64, made: usize) -> PortOrgRow {
        let capital = crate::port::def::tranches(capital_major)
            .into_iter()
            .take(made)
            .enumerate()
            .map(|(n, major)| TrancheRow {
                n: u32::try_from(n).expect("small"),
                obligation: 100 + u64::try_from(n).expect("small"),
                amount_minor: minor_of(major, CREDIT_SCALE).expect("fits"),
            })
            .collect();
        PortOrgRow {
            org: 1,
            root_scope: 1,
            capital_major,
            account: Some(1),
            capital,
            raises: Vec::new(),
        }
    }

    /// An authored org of `capital_major` with `raises` (doc 20 cut SM.5:
    /// `capital_drift` reads the authored org whole).
    fn authored(capital_major: u64, raises: &[u64]) -> crate::port::def::PortOrgDef {
        crate::port::def::PortOrgDef {
            id: CanonicalId::new("occator").expect("id"),
            name: "Occator Water & Propellant".to_owned(),
            capital_major,
            faction: CanonicalId::new("freeport").expect("id"),
            raises: raises.to_vec(),
        }
    }

    /// **A changed capital is drift even when the new tranches extend the
    /// recorded ones** (L4, D-18-27): 4,000,000 is five rows of 800,000, and
    /// 4,800,000 / 4,500,000 begin with the same five.
    #[test]
    fn ic3_capital_drift_refuses_a_top_up_and_completes_a_partial_apply() {
        let whole = row(4_000_000, 5);
        assert_eq!(
            capital_drift(&authored(4_000_000, &[]), &whole, CREDIT_SCALE),
            None
        );
        for capital in [4_800_000, 4_500_000, 3_200_000, 800_000] {
            assert_eq!(
                capital_drift(&authored(capital, &[]), &whole, CREDIT_SCALE),
                Some(CapitalDrift::Changed {
                    recorded: 4_000_000
                }),
                "{capital}"
            );
        }
        // The same capital with tranches still to make: a pass that stopped
        // part way, completed by the next one.
        assert_eq!(
            capital_drift(&authored(4_000_000, &[]), &row(4_000_000, 2), CREDIT_SCALE),
            None
        );
        assert_eq!(
            capital_drift(&authored(3_100_000, &[]), &row(3_100_000, 0), CREDIT_SCALE),
            None
        );
        // A damaged map: an amount that is not its tranche's, an index past
        // the list, a repeated index.
        let mut wrong = row(3_100_000, 4);
        wrong.capital[3].amount_minor = 80_000_000;
        let mut past = row(50_000, 1);
        past.capital[0].n = 1;
        let mut twice = row(4_000_000, 2);
        twice.capital[1].n = 0;
        twice.capital[1].amount_minor = twice.capital[0].amount_minor;
        for (name, damaged) in [("wrong", wrong), ("past", past), ("twice", twice)] {
            assert!(
                matches!(
                    capital_drift(
                        &authored(damaged.capital_major, &[]),
                        &damaged,
                        CREDIT_SCALE
                    ),
                    Some(CapitalDrift::Inconsistent { .. })
                ),
                "{name}"
            );
        }
    }

    /// **Doc 20 cut SM.5 (§7.3; reviewer F3)**: a raise appends tranches to the
    /// schedule; with it unrecorded or recorded there is no drift; an edited, dropped
    /// or reordered raise is `Raises`; only a changed base is `Changed`; a tranche row
    /// whose amount is not its schedule slot's is `Inconsistent`.
    #[test]
    fn sm5_a_raise_extends_the_schedule_and_an_edited_raise_is_drift() {
        // Occator at v10: 50,000, one tranche, no raise recorded.
        let v10 = row(50_000, 1);
        // v11 authors the raise: no drift, the second tranche is still to make.
        assert_eq!(
            capital_drift(&authored(50_000, &[250_000]), &v10, CREDIT_SCALE),
            None
        );
        // The raise begun and recorded.
        let mut v11 = row(50_000, 1);
        v11.raises = vec![250_000];
        v11.capital.push(TrancheRow {
            n: 1,
            obligation: 200,
            amount_minor: minor_of(250_000, CREDIT_SCALE).expect("fits"),
        });
        assert_eq!(
            capital_drift(&authored(50_000, &[250_000]), &v11, CREDIT_SCALE),
            None
        );
        // A further raise appended: still no drift.
        assert_eq!(
            capital_drift(&authored(50_000, &[250_000, 100_000]), &v11, CREDIT_SCALE),
            None
        );
        // Edited, dropped, reordered: `Raises`.
        for raises in [&[200_000_u64][..], &[], &[100_000, 250_000]] {
            assert_eq!(
                capital_drift(&authored(50_000, raises), &v11, CREDIT_SCALE),
                Some(CapitalDrift::Raises),
                "{raises:?}"
            );
        }
        // A changed base is `Changed`, raise or no raise.
        assert_eq!(
            capital_drift(&authored(60_000, &[250_000]), &v11, CREDIT_SCALE),
            Some(CapitalDrift::Changed { recorded: 50_000 })
        );
        // A tranche row whose amount is not its schedule slot's.
        let mut wrong = v11.clone();
        wrong.capital[1].amount_minor = 1;
        assert_eq!(
            capital_drift(&authored(50_000, &[250_000]), &wrong, CREDIT_SCALE),
            Some(CapitalDrift::Inconsistent { recorded: 50_000 })
        );
        // The raise's tranche recorded with the raise left unauthored: the index is past
        // the base's schedule.
        let mut orphan = row(50_000, 1);
        orphan.capital.push(v11.capital[1].clone());
        assert_eq!(
            capital_drift(&authored(50_000, &[]), &orphan, CREDIT_SCALE),
            Some(CapitalDrift::Inconsistent { recorded: 50_000 })
        );
    }

    /// Doc 18 §4.7 / cut IC6: the leased-hull retire refusal names the entry
    /// and the mark.
    #[test]
    fn ic6_the_leased_retire_refusal_names_the_hull() {
        let id = CanonicalId::new("ceres_port").expect("id");
        assert_eq!(
            leased_refusal(&id, "FPC-D-0401"),
            "ceres_port: hull FPC-D-0401 is chartered to a pilot; a port with a leased hull \
             cannot be retired"
        );
    }
}
