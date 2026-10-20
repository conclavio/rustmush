//! **The `PortApplier`'s parts d–e** (doc 18 cut IC8; doc 18 §4.7 "(d) the goods
//! catalog", "(e) vendors and stock", §4.3 "Vendors and buyers"; lead rulings L2,
//! L3, L6, L15, L25).
//!
//! For a `port` entry that authors vendors, the applier — on the content module's
//! context, under the latch, **submitting nothing** — makes, once, and records each
//! act in the port map before the next:
//!
//! - **(d)** the goods catalog: `mush_market::load_pack(tacsim.goods, ..)` on every
//!   apply (byte-identical entries are no-ops);
//! - **(e)**, per vendor in authored order: the port's **stock clerk** (one Thing per
//!   port, `DARK`, in the staff spine, owned by the applying content owner — the hull
//!   Things' owner rule; lead ruling L2), the **board rooms** (lead ruling L15), the
//!   authored **storefront** item and **signatory** npc (resolved by name in their
//!   authored room at first apply, by map row ever after: lead ruling L3), the
//!   **storefront vault** (org-owned, in the org's root scope, backed by the
//!   storefront container), the market **vendor**, the clerk's **direct
//!   `vault.withdraw` grant** in the org's root scope (lead ruling L25: `authorize`
//!   honours a direct grant with no membership), the **stock plan rows**, and the
//!   **standing bids** (`mush_market::post_wanted`). An update re-prices an ask by
//!   withdrawing its Draw listing (the stock arm relists), and revises a changed bid
//!   in place. **The applier makes no Draw listing**: the stock arm lists once the
//!   pool exists.
//!
//! The economy and org allocators are reached through `apply.rs`'s two helpers
//! ([`crate::port::apply`]'s `alloc_vault_id` / `alloc_grant_id`), so the state-slot
//! gate's rows for `apply.rs` cover them.

use std::collections::BTreeSet;

use mush_atlas::{AtlasState, RecordKey};
use mush_core::{CanonicalId, Ctx, Flags, ObjId, ObjKind, World};
use mush_economy::{
    Amount, CatalogRef, CustodyRef, EconomyStore, InstrumentId, Vault, VaultId, VaultStatus,
};
use mush_market::{ListingId, LotMatter, MarketId, MarketStore, VendorId, WantedSpec};
use mush_org::{
    CapabilityGrant, CapabilityId, OrgAction, OrgId, OrgMutationOp, OrgMutationPlan, OrgScopeId,
    PrincipalId, ScopeSelector,
};
use smol_str::SmolStr;

use crate::economy::CREDIT;
use crate::port::apply::{alloc_grant_id, alloc_vault_id, credit_scale, minor_of, room_by_key};
use crate::port::coop::port_name;
use crate::port::def::{PortDef, room_key};
use crate::port::families::StockIssueId;
use crate::port::map::{BidRow, ClerkRow, PortMap, StockRow, VendorRow};
use crate::store::put_port_map;

/// The capability the clerk is granted on each storefront (lead ruling L25).
pub const CLERK_CAPABILITY: &str = "vault.withdraw";

/// **An authored item or npc of a station pack, as a live object** (lead ruling L3):
/// the reference split by [`room_key`]; the entry read from the station pack on the
/// atlas shelf (its `kind` must be `kind`, its `def.name` and `def.room` read); the
/// authored room found by its `RoomGeo.key`; and the **unique** live object of the
/// matching kind (`Thing` for an item, `Npc` for an npc) standing in that room under
/// that name. Zero or several matches is a refusal. Used at first apply only — every
/// later apply reads the map row (IC3's "by map row, never by name").
///
/// # Errors
///
/// A sentence naming the reference and what did not resolve.
pub(crate) fn authored_object(
    w: &World,
    area: &CanonicalId,
    reference: &str,
    kind: &str,
) -> Result<ObjId, String> {
    let record = room_key(area, reference)?;
    let atlas = w
        .module_state::<AtlasState>()
        .ok_or_else(|| "the atlas is not composed on this world".to_owned())?;
    let pack = atlas
        .shelf
        .get(area)
        .ok_or_else(|| format!("the station pack {area} is not on the atlas shelf"))?;
    let entry = pack
        .entry(&record.entry)
        .ok_or_else(|| format!("{reference} is not an entry of {area}"))?;
    if entry.kind().as_str() != kind {
        return Err(format!("{reference} is not an {kind} entry"));
    }
    let field = |name: &str| {
        entry
            .def()
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(|s| s.trim().to_owned())
            .ok_or_else(|| format!("{reference} has no {name}"))
    };
    let name = field("name")?;
    let room_entry = CanonicalId::parse_canonical(&field("room")?)
        .map_err(|_| format!("{reference} names no room of {area}"))?;
    let room = room_by_key(w, &RecordKey::new(area.clone(), room_entry))
        .ok_or_else(|| format!("{reference}'s room is not on this world"))?;
    let want = if kind == "npc" {
        ObjKind::Npc
    } else {
        ObjKind::Thing
    };
    let mut found = w.contents_of(room).iter().copied().filter(|obj| {
        w.get(*obj)
            .is_some_and(|o| o.kind == want && o.name == name && !o.flags.contains(Flags::GOING))
    });
    let first = found
        .next()
        .ok_or_else(|| format!("{reference} ({name}) is not in its room on this world"))?;
    if found.next().is_some() {
        return Err(format!(
            "{reference} ({name}) is not unique in its room on this world"
        ));
    }
    Ok(first)
}

/// Whether `obj` is live: present and not `GOING`.
fn live(w: &World, obj: u32) -> bool {
    w.get(ObjId(obj))
        .is_some_and(|o| !o.flags.contains(Flags::GOING))
}

/// The room a role names in this entry, by its `RoomGeo.key`. `pub(crate)` since doc
/// 20 cut SM.3a (reviewer F14): `+tv/lease` finds Occator's counter through it.
pub(crate) fn role_room(w: &World, def: &PortDef, role: &CanonicalId) -> Option<ObjId> {
    let reference = def.rooms.get(role)?;
    room_by_key(w, &room_key(&def.area, reference).ok()?)
}

/// Every board role the entry's asks and bids name, in first-named order.
fn board_roles(def: &PortDef) -> Vec<CanonicalId> {
    let mut roles: Vec<CanonicalId> = Vec::new();
    for vendor in &def.vendors {
        let named = vendor
            .asks
            .iter()
            .flat_map(|ask| ask.boards.iter())
            .chain(vendor.bids.iter().flat_map(|bid| bid.boards.iter()));
        for role in named {
            if !roles.contains(role) {
                roles.push(role.clone());
            }
        }
    }
    roles
}

/// **Whether part (e).8 posts `bid` afresh on this apply**: it is unrecorded, or its
/// recorded listing is gone past the deadline it was posted with and its intake is
/// unchanged (it lapsed). One gone before its deadline was withdrawn by hand — since
/// doc 18 cut IC10 an exhausted bid is no longer deleted but stands at quantity 0 —
/// and is not re-posted until that deadline. The one rule `preflight`, `put`, the
/// stock clerk's bid re-post (cut IC10) and the readiness Bids line all read. A gone
/// bid whose authored intake changed is not posted by the apply itself: `put`
/// records the new terms in its `BidRow` (cut IC10 fixer, review F2), after which
/// this rule holds again and the clerk posts it once the deadline has passed.
pub(crate) fn posts_afresh(
    w: &World,
    recorded: Option<&BidRow>,
    bid: &crate::port::def::BidDef,
) -> bool {
    let Some(recorded) = recorded else {
        return true;
    };
    let standing = w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.listing(ListingId(recorded.listing)).is_some());
    !standing && w.now() >= recorded.expires && recorded.intake == bid.intake
}

/// The `tacsim.credit` instrument.
fn credit() -> Result<InstrumentId, String> {
    InstrumentId::new(CREDIT).ok_or_else(|| format!("{CREDIT} is not an instrument id"))
}

/// The catalogue reference of one good (ruleset `tacsim`, code = the good id).
fn catalog_of(good: &CanonicalId) -> CatalogRef {
    CatalogRef {
        ruleset: SmolStr::new_static(crate::NS),
        code: SmolStr::new(good.as_str()),
    }
}

/// **A storefront item that already backs a vault is refused** (doc 18 cut IC14 row
/// 30): the economy keeps one vault per backing (`@vault/create`'s check), so an
/// unrecorded vendor whose storefront item already backs a vault would make a second
/// vault the snapshot's backing index cannot hold.
fn backs_no_vault(
    w: &World,
    id: &CanonicalId,
    org: &CanonicalId,
    storefront: ObjId,
) -> Result<(), String> {
    let backing = w.module_state::<EconomyStore>().and_then(|economy| {
        economy
            .snapshot()
            .vault_by_backing(&CustodyRef::Container(storefront))
            .map(|v| v.id.0)
    });
    match backing {
        Some(n) => Err(format!(
            "{id}: vendor {org}: storefront: that item already backs vault #{n}"
        )),
        None => Ok(()),
    }
}

/// **Parts d–e's refusals, before any write** (doc 18 cut IC8; the `PortApplier`'s
/// preflight, after its IC3/IC4 drift checks): the market is composed; every stock, ask
/// and bid good has a catalogue entry `catalog_entries()` derives; each storefront and
/// signatory resolves (unrecorded) or is live (recorded), the storefront in the staff
/// spine; the recorded clerk is live; every board role resolves; no recorded stock row
/// is changed or dropped (lead ruling L6, paged); every price converts, and every bid
/// posted afresh has an in-range intake, no unrecorded twin and room under the
/// vendor's listing cap (fixer F3); no recorded bid is dropped, and no revision would
/// take a bid's remaining intake past `MAX_QUANTITY` (cut IC10: a cut below what is
/// left saturates at 0, the exhausted bid); no unrecorded storefront already backs a
/// vault ([`backs_no_vault`], cut IC14 row 30).
///
/// # Errors
///
/// The first refusal, as a sentence naming the entry.
#[allow(clippy::too_many_lines)]
pub(crate) fn preflight(
    w: &World,
    id: &CanonicalId,
    def: &PortDef,
    map: Option<&PortMap>,
) -> Result<(), String> {
    if def.vendors.is_empty() {
        return Ok(());
    }
    let Some(market) = w.module_state::<MarketStore>() else {
        return Err(format!("{id}: the market is not composed on this world"));
    };
    // 2. Every good has a catalogue entry the goods table derives.
    let goods = crate::definitions::goods::goods_table()
        .map_err(|error| format!("{id}: the goods table: {error}"))?;
    let entries = goods
        .catalog_entries()
        .map_err(|error| format!("{id}: the goods catalog: {error}"))?;
    let codes: BTreeSet<&str> = entries.iter().map(|e| e.catalog.code.as_str()).collect();
    for vendor in &def.vendors {
        let named = vendor
            .stock
            .iter()
            .map(|row| &row.good)
            .chain(vendor.asks.iter().map(|ask| &ask.good))
            .chain(vendor.bids.iter().map(|bid| &bid.good));
        for good in named {
            if !codes.contains(good.as_str()) {
                return Err(format!(
                    "{id}: vendor {}: {good} has no catalogue entry",
                    vendor.org
                ));
            }
        }
    }
    // 2b. The figures part (e) will write (fixer F3: refused here, before the clerk,
    // the vaults, the vendors and the grants are made, never inside `put`): every ask
    // and bid price converts to minor units; every bid that this apply posts afresh
    // has an intake the market takes, no live bid of its vendor already standing for
    // its good, and room under the vendor's listing cap.
    let scale = credit_scale(w);
    for vendor in &def.vendors {
        let org = &vendor.org;
        for (side, good, major) in vendor
            .asks
            .iter()
            .map(|ask| ("ask", &ask.good, ask.price_major))
            .chain(
                vendor
                    .bids
                    .iter()
                    .map(|bid| ("bid", &bid.good, bid.price_major)),
            )
        {
            if minor_of(major, scale).is_none_or(|minor| minor <= 0) {
                return Err(format!(
                    "{id}: vendor {org} {side} {good}: the price overflows"
                ));
            }
        }
        let row = map.and_then(|m| m.vendors.get(org));
        let vendor_id = row.and_then(|row| row.vendor).map(VendorId);
        let mut posts: usize = 0;
        for bid in &vendor.bids {
            if !posts_afresh(w, row.and_then(|row| row.bids.get(&bid.good)), bid) {
                continue;
            }
            if bid.intake == 0 || bid.intake > mush_market::MAX_QUANTITY {
                return Err(format!(
                    "{id}: vendor {org} bid {}: an intake of {} is not a quantity the market buys",
                    bid.good, bid.intake
                ));
            }
            if vendor_id.is_some_and(|v| market.wanted_of(v, &bid.good).is_some()) {
                return Err(format!(
                    "{id}: vendor {org} bid {}: a standing bid the port map does not record \
                     already stands for it; withdraw it or repair the map by hand",
                    bid.good
                ));
            }
            posts = posts.saturating_add(1);
        }
        let holding = vendor_id.map_or(0, |v| market.listings_of_vendor(v));
        if holding.saturating_add(posts) > mush_market::MAX_LISTINGS_PER_VENDOR {
            return Err(format!(
                "{id}: vendor {org}: posting {posts} bids would pass the market's {} listings per \
                 vendor",
                mush_market::MAX_LISTINGS_PER_VENDOR
            ));
        }
    }
    // 4. The clerk, the storefronts and the signatories.
    if let Some(clerk) = map.and_then(|m| m.clerk.as_ref())
        && !live(w, clerk.thing)
    {
        return Err(format!(
            "{id}: the stock clerk #{} in the port map is gone; it is never re-created — repair \
             the map by hand",
            clerk.thing
        ));
    }
    let spine = def
        .rooms
        .iter()
        .find(|(role, _)| role.as_str() == "spine")
        .map(|(role, _)| role.clone())
        .and_then(|role| role_room(w, def, &role));
    for vendor in &def.vendors {
        let org = &vendor.org;
        match map.and_then(|m| m.vendors.get(org)) {
            Some(row) => {
                for (what, obj) in [("storefront", row.storefront), ("signatory", row.signatory)] {
                    if !live(w, obj) {
                        return Err(format!(
                            "{id}: vendor {org}: the {what} #{obj} in the port map is gone; it is \
                             never re-created — repair the map by hand"
                        ));
                    }
                }
                let vault_ok = w
                    .module_state::<EconomyStore>()
                    .and_then(|e| e.snapshot().vault(VaultId(row.vault)).cloned())
                    .is_some_and(|v| v.status == VaultStatus::Active);
                if !vault_ok {
                    return Err(format!(
                        "{id}: vendor {org}: storefront vault #{} in the port map is not active; it \
                         is never re-created — repair the map by hand",
                        row.vault
                    ));
                }
            }
            None => {
                let storefront = authored_object(w, &def.area, &vendor.storefront, "item")
                    .map_err(|detail| format!("{id}: vendor {org}: storefront: {detail}"))?;
                let room = w.get(storefront).and_then(|o| o.location);
                if spine.is_none() || room != spine {
                    return Err(format!(
                        "{id}: vendor {org}: the storefront {} is not in the port's staff spine",
                        vendor.storefront
                    ));
                }
                // Doc 18 cut IC14 row 30: `@vault/create`'s one-vault-per-backing rule,
                // refused here before any write rather than discovered inside `put`.
                backs_no_vault(w, id, org, storefront)?;
                authored_object(w, &def.area, &vendor.signatory, "npc")
                    .map_err(|detail| format!("{id}: vendor {org}: signatory: {detail}"))?;
            }
        }
    }
    // 5. The board rooms.
    for role in board_roles(def) {
        match map.and_then(|m| m.boards.get(&role)) {
            Some(room) if live(w, *room) => {}
            Some(room) => {
                return Err(format!(
                    "{id}: board room `{role}` #{room} in the port map is gone; repair the map by \
                     hand"
                ));
            }
            None => {
                if role_room(w, def, &role).is_none() {
                    return Err(format!("{id}: board room `{role}` is not on this world"));
                }
            }
        }
    }
    let Some(map) = map else {
        return Ok(());
    };
    // 6. Recorded stock-row drift (lead ruling L6, owner-critical: the map row is the
    // record, since the applier cannot read the receipts from the content context).
    for (org, row) in &map.vendors {
        let authored = def.vendors.iter().find(|v| &v.org == org);
        for recorded in &row.stock {
            let now = authored.and_then(|v| v.stock.iter().find(|s| s.good == recorded.good));
            let same = now.is_some_and(|s| {
                s.issue == recorded.issue && s.qty == recorded.qty && s.grade == recorded.grade
            });
            if !same {
                let sentence = format!(
                    "{id}: vendor {org} stock {}: issue {} is already recorded; a port update never \
                     re-issues or re-sizes stock (Q-18-19)",
                    recorded.good, recorded.issue
                );
                tracing::error!(
                    runbook = "@tacsim/port status; keep the recorded stock row in the pack",
                    "{sentence}"
                );
                return Err(sentence);
            }
        }
        // 7. Bid drift: a recorded bid is never retired by an update, and a revision
        // keeps the remaining intake in range. Doc 18 cut IC10 (decision 22): a cut of
        // the authored intake by more than is left saturates at 0 — the bid stands
        // exhausted — the same arithmetic the revise arm in `put` makes.
        for (good, bid) in &row.bids {
            let Some(now) = authored.and_then(|v| v.bids.iter().find(|b| &b.good == good)) else {
                return Err(format!(
                    "{id}: vendor {org} bid {good} is in the port map but no longer authored; bids \
                     are never retired by a port update"
                ));
            };
            if now.intake == bid.intake && now.price_major == bid.price_major {
                continue;
            }
            if let Some(listing) = market.listing(ListingId(bid.listing)) {
                let revised = listing
                    .quantity
                    .checked_add(now.intake)
                    .map(|sum| sum.saturating_sub(bid.intake));
                if !revised.is_some_and(|r| r <= mush_market::MAX_QUANTITY) {
                    return Err(format!(
                        "{id}: vendor {org} bid {good}: an intake of {} would leave the bid's \
                         remaining intake out of range",
                        now.intake
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The vault's org root scope, from the map's org row.
fn org_row(
    map: &PortMap,
    id: &CanonicalId,
    org: &CanonicalId,
) -> Result<(OrgId, OrgScopeId), String> {
    map.orgs
        .get(org)
        .map(|row| (OrgId(row.org), OrgScopeId(row.root_scope)))
        .ok_or_else(|| format!("{id}: vendor {org} has no org row"))
}

/// **Parts d–e** (doc 18 §4.7; cut IC8), in order, every act recorded in `map` and the
/// map written immediately after it. Normal-lane writes only; nothing is submitted.
///
/// # Errors
///
/// A sentence naming the entry and the act that refused.
#[allow(clippy::too_many_lines)]
pub(crate) fn put(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    def: &PortDef,
    map: &mut PortMap,
) -> Result<(), String> {
    if def.vendors.is_empty() {
        return Ok(());
    }
    // (d) The goods catalog, every apply (idempotent).
    let goods = crate::definitions::goods::goods_table()
        .map_err(|error| format!("{id}: the goods table: {error}"))?;
    let entries = goods
        .catalog_entries()
        .map_err(|error| format!("{id}: the goods catalog: {error}"))?;
    mush_market::load_pack(ctx, crate::definitions::goods::GOODS_PACK, entries)
        .map_err(|error| format!("{id}: the goods catalog: {error:?}"))?;
    let instrument = credit()?;
    let venue = MarketId::new(def.venue.as_str())
        .ok_or_else(|| format!("{id}: venue {} is not a market id", def.venue))?;
    let scale = credit_scale(ctx.w);
    let spine = def
        .rooms
        .keys()
        .find(|role| role.as_str() == "spine")
        .cloned()
        .and_then(|role| role_room(ctx.w, def, &role))
        .ok_or_else(|| format!("{id}: the port's staff spine is not on this world"))?;
    // (e).1 The clerk, once per port (lead ruling L2).
    if map.clerk.is_none() {
        let agent = def
            .vendors
            .first()
            .map(|vendor| vendor.agent.clone())
            .ok_or_else(|| format!("{id}: no vendor names a clerk"))?;
        let name = format!("{} stock clerk", port_name(def));
        let thing = ctx
            .w
            .create(ObjKind::Thing, &name, ctx.actor, Some(spine))
            .map_err(|e| format!("{id}: the stock clerk: {e}"))?;
        ctx.w
            .set_flag(thing, Flags::DARK, true)
            .map_err(|e| format!("{id}: the stock clerk: {e}"))?;
        map.clerk = Some(ClerkRow {
            agent,
            thing: thing.0,
        });
        put_port_map(ctx, map);
    }
    let clerk = map
        .clerk
        .as_ref()
        .map(|c| ObjId(c.thing))
        .ok_or_else(|| format!("{id}: no clerk is recorded"))?;
    // (e).2 The board rooms (lead ruling L15).
    for role in board_roles(def) {
        if let std::collections::btree_map::Entry::Vacant(slot) = map.boards.entry(role) {
            let room = role_room(ctx.w, def, slot.key())
                .ok_or_else(|| format!("{id}: board room `{}` is not on this world", slot.key()))?;
            slot.insert(room.0);
            put_port_map(ctx, map);
        }
    }
    for vendor in &def.vendors {
        let org = &vendor.org;
        let (org_id, root_scope) = org_row(map, id, org)?;
        let org_name = def
            .orgs
            .iter()
            .find(|o| &o.id == org)
            .map_or_else(|| org.to_string(), |o| o.name.clone());
        // (e).3–4 Storefront, signatory and the storefront vault (unrecorded only).
        if !map.vendors.contains_key(org) {
            let storefront = authored_object(ctx.w, &def.area, &vendor.storefront, "item")
                .map_err(|detail| format!("{id}: vendor {org}: storefront: {detail}"))?;
            let signatory = authored_object(ctx.w, &def.area, &vendor.signatory, "npc")
                .map_err(|detail| format!("{id}: vendor {org}: signatory: {detail}"))?;
            // Doc 18 cut IC14 row 30: the preflight's rule, re-run where the vault is made.
            backs_no_vault(ctx.w, id, org, storefront)?;
            let vault = alloc_vault_id(ctx).map_err(|e| format!("{id}: vendor {org}: {e}"))?;
            mush_economy::create_vault(
                ctx,
                Vault {
                    id: vault,
                    owner: PrincipalId::Organization(org_id),
                    scope: Some(root_scope),
                    name: SmolStr::new(format!("{org_name} storefront")),
                    backing: CustodyRef::Container(storefront),
                    status: VaultStatus::Active,
                    revision: 1,
                },
            );
            map.vendors.insert(
                org.clone(),
                VendorRow {
                    storefront: storefront.0,
                    vault: vault.0,
                    vendor: None,
                    signatory: signatory.0,
                    grant: None,
                    stock: Vec::new(),
                    bids: std::collections::BTreeMap::new(),
                },
            );
            put_port_map(ctx, map);
        }
        let Some(row) = map.vendors.get(org).cloned() else {
            return Err(format!("{id}: vendor {org} has no map row"));
        };
        let vault = VaultId(row.vault);
        // (e).5 The market vendor, when unrecorded or when the market lost it.
        let registered = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|s| s.vendor_of_org(org_id).map(|v| v.id));
        let vendor_id = match (row.vendor, registered) {
            (Some(recorded), Some(live)) if recorded == live.0 => live,
            _ => {
                let vendor_id = mush_market::register_vendor(
                    ctx,
                    &instrument,
                    org_id,
                    vault,
                    venue.clone(),
                    SmolStr::new(&org_name),
                    0,
                )
                .map_err(|e| format!("{id}: vendor {org}: the market vendor: {e:?}"))?;
                if let Some(row) = map.vendors.get_mut(org) {
                    row.vendor = Some(vendor_id.0);
                }
                put_port_map(ctx, map);
                vendor_id
            }
        };
        // (e).6 The clerk's direct `vault.withdraw` grant (lead ruling L25).
        if row.grant.is_none() {
            let grant_id = alloc_grant_id(ctx).map_err(|e| format!("{id}: vendor {org}: {e}"))?;
            let capability = CapabilityId::new(CLERK_CAPABILITY)
                .ok_or_else(|| format!("{id}: {CLERK_CAPABILITY} is not a capability id"))?;
            let plan = OrgMutationPlan::of(vec![OrgMutationOp::PutGrant {
                grant: CapabilityGrant {
                    id: grant_id,
                    org: org_id,
                    grantee: clerk,
                    capability,
                    scope: ScopeSelector::Exact(root_scope),
                    limit: None,
                    delegable: false,
                    valid_from: None,
                    valid_until: None,
                    revision: 1,
                },
            }]);
            mush_org::submit(
                ctx,
                &plan,
                OrgAction {
                    action: SmolStr::new_static("grant.create"),
                    org: Some(org_id),
                    detail: serde_json::json!({
                        "port": format!("{}/{id}", map.pack),
                        "org": org.to_string(),
                        "clerk": clerk.0,
                    }),
                },
            )
            .map_err(|e| format!("{id}: vendor {org}: the clerk's grant: {e}"))?;
            if let Some(row) = map.vendors.get_mut(org) {
                row.grant = Some(grant_id.0);
            }
            put_port_map(ctx, map);
        }
        // (e).7 The stock plan rows not yet recorded.
        for stock in &vendor.stock {
            let recorded = map
                .vendors
                .get(org)
                .is_some_and(|row| row.stock.iter().any(|s| s.good == stock.good));
            if recorded {
                continue;
            }
            let issue = StockIssueId::derive(&map.pack, &map.entry, org, &stock.good, stock.issue)
                .map_err(|e| format!("{id}: vendor {org} stock {}: {e}", stock.good))?;
            if let Some(row) = map.vendors.get_mut(org) {
                row.stock.push(StockRow {
                    good: stock.good.clone(),
                    issue: stock.issue,
                    qty: stock.qty,
                    grade: stock.grade,
                    id: issue.key_text(),
                    lot: issue.lot_id().0,
                });
            }
            put_port_map(ctx, map);
        }
        // (e).8 The standing bids.
        for bid in &vendor.bids {
            // Doc 19 cut A.4 (D-19-14): the port's reach premium rides the posting.
            let price = minor_of(crate::port::price::bid_price_major(ctx.w, def, bid), scale)
                .map(Amount::from_minor)
                .ok_or_else(|| {
                    format!("{id}: vendor {org} bid {}: the price overflows", bid.good)
                })?;
            let recorded = map
                .vendors
                .get(org)
                .and_then(|row| row.bids.get(&bid.good))
                .cloned();
            let standing = recorded.as_ref().and_then(|r| {
                ctx.w
                    .module_state::<MarketStore>()
                    .and_then(|s| s.listing(ListingId(r.listing)).cloned())
            });
            match (recorded, standing) {
                (Some(r), Some(listing)) => {
                    if r.price_major == bid.price_major && r.intake == bid.intake {
                        continue;
                    }
                    // Doc 18 cut IC10 (decision 22): an authored intake that falls by
                    // more than is left saturates at 0 — the bid stands exhausted —
                    // rather than failing the whole apply, which before IC10 could not
                    // happen to an exhausted bid because it was deleted. Overflow of the
                    // add still errors.
                    let remaining = listing
                        .quantity
                        .checked_add(bid.intake)
                        .map(|sum| sum.saturating_sub(r.intake))
                        .ok_or_else(|| {
                            format!(
                                "{id}: vendor {org} bid {}: the intake is out of range",
                                bid.good
                            )
                        })?;
                    mush_market::revise_wanted(ctx, listing.id, price, remaining)
                        .map_err(|e| format!("{id}: vendor {org} bid {}: {e:?}", bid.good))?;
                    if let Some(row) = map.vendors.get_mut(org) {
                        row.bids.insert(
                            bid.good.clone(),
                            BidRow {
                                price_major: bid.price_major,
                                intake: bid.intake,
                                ..r
                            },
                        );
                    }
                    put_port_map(ctx, map);
                }
                (Some(r), None) => {
                    // Gone. Re-posted only when it lapsed with its intake unchanged;
                    // one gone before its deadline was withdrawn by hand (cut IC10: an
                    // exhausted bid stands at 0 and is never gone).
                    if posts_afresh(ctx.w, Some(&r), bid) {
                        // Doc 20 cut SM.5 (decision 20; the lead's pre-commit round,
                        // item 2): a lapsed REFRESH row comes back at one drip, within
                        // the house's funds, through the drip's own rule — never at its
                        // full authored intake — and an unfunded house's is left for the
                        // drip, which warns. Every other bid re-posts its full intake.
                        let quantity = match bid.refresh {
                            Some(refresh) => {
                                let fundable = ctx.w.module_state::<MarketStore>().map_or(0, |m| {
                                    crate::port::board::vendor_fundable_units(
                                        ctx.w, m, vendor_id, price,
                                    )
                                });
                                crate::port::stock::lapsed_drip(refresh, fundable)
                            }
                            None => bid.intake,
                        };
                        if quantity > 0 {
                            post_bid(ctx, id, map, org, vendor_id, vault, bid, price, quantity)?;
                        }
                        continue;
                    }
                    // Doc 18 cut IC10 (fixer, review F2): a gone bid whose authored
                    // terms changed records them, keeping its listing and deadline, so
                    // the stock clerk posts it at the new intake once that deadline has
                    // passed (`posts_afresh` reads the recorded intake) — never a bid
                    // that no apply and no clerk will ever post again.
                    if r.price_major != bid.price_major || r.intake != bid.intake {
                        if let Some(row) = map.vendors.get_mut(org) {
                            row.bids.insert(
                                bid.good.clone(),
                                BidRow {
                                    price_major: bid.price_major,
                                    intake: bid.intake,
                                    ..r
                                },
                            );
                        }
                        put_port_map(ctx, map);
                    }
                }
                (None, _) => {
                    post_bid(ctx, id, map, org, vendor_id, vault, bid, price, bid.intake)?;
                }
            }
        }
        // (e).9 Ask re-pricing: a live Draw listing at another price is withdrawn (a
        // Draw stages nothing); the stock arm relists at the authored price.
        for ask in &vendor.asks {
            let Some(price) = minor_of(ask.price_major, scale).map(Amount::from_minor) else {
                return Err(format!(
                    "{id}: vendor {org} ask {}: the price overflows",
                    ask.good
                ));
            };
            let stale = ctx.w.module_state::<MarketStore>().and_then(|s| {
                let listing = s.listing(
                    s.lot_listing_of(mush_inventory::record::EndpointRef(vault), &ask.good)?,
                )?;
                let drawn = matches!(s.lot_matter(listing), Some(LotMatter::Draw { .. }));
                (drawn && listing.price != price).then_some(listing.id)
            });
            if let Some(listing) = stale {
                mush_market::withdraw_listing(ctx, listing)
                    .map_err(|e| format!("{id}: vendor {org} ask {}: {e:?}", ask.good))?;
            }
        }
    }
    Ok(())
}

/// **The one `WantedSpec` a port bid posts with** (doc 18 cut IC8's part (e).8,
/// factored by cut IC10 so the applier and the stock clerk's re-post build it one
/// way): the authored good, price and full intake, into the storefront, no grade
/// floor, for the market's longest listing life.
pub(crate) fn wanted_spec(
    vendor: VendorId,
    vault: VaultId,
    bid: &crate::port::def::BidDef,
    price: Amount,
) -> WantedSpec {
    WantedSpec {
        vendor,
        catalog: catalog_of(&bid.good),
        price,
        remaining: bid.intake,
        vault,
        min_grade: 0,
        ttl_secs: mush_market::MAX_LISTING_TTL_SECS,
    }
}

/// Post one standing bid of `quantity` and record it at its authored terms (the row's
/// `intake` is always the authored one, which the applier compares; `quantity` is the
/// full intake but for a lapsed refresh row's one drip, doc 20 cut SM.5).
#[allow(clippy::too_many_arguments)]
fn post_bid(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    map: &mut PortMap,
    org: &CanonicalId,
    vendor: VendorId,
    vault: VaultId,
    bid: &crate::port::def::BidDef,
    price: Amount,
    quantity: u64,
) -> Result<(), String> {
    let posted = crate::port::def::BidDef {
        intake: quantity,
        ..bid.clone()
    };
    let listing = mush_market::post_wanted(ctx, wanted_spec(vendor, vault, &posted, price))
        .map_err(|e| format!("{id}: vendor {org} bid {}: {e:?}", bid.good))?;
    let expires = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(listing).map(|l| l.expires.at))
        .unwrap_or_else(|| {
            ctx.w
                .now()
                .saturating_add(mush_market::MAX_LISTING_TTL_SECS)
        });
    if let Some(row) = map.vendors.get_mut(org) {
        row.bids.insert(
            bid.good.clone(),
            BidRow {
                listing: listing.0,
                price_major: bid.price_major,
                intake: bid.intake,
                expires,
                // Doc 20 cut SM.5: the IC instant of the post (the intake drip's
                // boundary is measured from it).
                posted_ic: Some(ctx.w.world_ic_now().at),
            },
        );
    }
    put_port_map(ctx, map);
    Ok(())
}

/// **The vendors' retire refusal** (doc 18 cut IC8): a vendor of the port's map with
/// an open market deal — a quote with a contract and no settlement, sold by the
/// vendor's org, or (doc 18 cut IC10) **bought** by it: a sale into one of its bids in
/// progress — keeps the port from retiring.
pub(crate) fn retire_refusal(w: &World, id: &CanonicalId, map: &PortMap) -> Result<(), String> {
    let Some(market) = w.module_state::<MarketStore>() else {
        return Ok(());
    };
    for org in map.vendors.keys() {
        let Some(row) = map.orgs.get(org) else {
            continue;
        };
        let seller = PrincipalId::Organization(OrgId(row.org));
        if market.quotes().any(|q| {
            q.contract.is_some() && q.settled.is_none() && (q.seller == seller || q.buyer == seller)
        }) {
            return Err(format!(
                "{id}: vendor {org} has a deal in progress; a port with live contracts cannot be \
                 retired"
            ));
        }
    }
    Ok(())
}

/// **The vendors' half of retire** (doc 18 §4.7 "Retire withdraws asks and bids"; cut
/// IC8, reviewer F19): every vendor's **Draw** listings and **standing bids** are
/// withdrawn (neither stages anything). Any other listing a vendor holds — a
/// staff-stocked hold-backed `Supply::Lot`, say — is left standing and logged, because
/// withdrawing it would stage a release from the applier. Matter, receipts, orgs,
/// accounts, vaults, vendors, the clerk and the hulls all stand.
///
/// Since doc 18 cut IC10 the market's live set holds an **exhausted** bid (quantity 0)
/// too, so a retire withdraws it and forgets its row like any other; a re-apply then
/// posts it afresh at its authored intake (owner item O4(c)).
///
/// **A withdrawn bid's row leaves the map** (reviewer F2 of the IC8 fixer pass): a bid
/// the retire took down is not one whose intake was taken, so the map must not keep a
/// `BidRow` that a later apply would read as "gone before its deadline — exhausted,
/// not buying". With the row gone, the next apply posts the bid afresh (part (e).8's
/// unrecorded arm). The map is written once, after the withdrawals.
pub(crate) fn retire_listings(ctx: &mut Ctx<'_>, id: &CanonicalId, map: &mut PortMap) {
    let vendors: Vec<u64> = map.vendors.values().filter_map(|row| row.vendor).collect();
    let mut withdraw: Vec<ListingId> = Vec::new();
    if let Some(market) = ctx.w.module_state::<MarketStore>() {
        for listing in market.live_listings() {
            if !vendors.contains(&listing.vendor.0) {
                continue;
            }
            let drawn = matches!(market.lot_matter(listing), Some(LotMatter::Draw { .. }));
            if drawn || listing.source.is_wanted() {
                withdraw.push(listing.id);
            } else {
                tracing::warn!(
                    %id, listing = listing.id.0,
                    "tacsim: a retiring port vendor's listing is not a Draw or a bid; it is left \
                     standing for staff to withdraw"
                );
            }
        }
    }
    let mut withdrawn: Vec<u64> = Vec::new();
    for listing in withdraw {
        match mush_market::withdraw_listing(ctx, listing) {
            Ok(()) => withdrawn.push(listing.0),
            Err(error) => {
                tracing::warn!(%id, listing = listing.0, ?error, "tacsim: a retire withdrawal refused");
            }
        }
    }
    // Forget exactly the recorded bids this retire withdrew; a row whose listing was
    // already gone (its intake taken, or lapsed) keeps the reading it had.
    let mut forgot = false;
    for row in map.vendors.values_mut() {
        let before = row.bids.len();
        row.bids.retain(|_, bid| !withdrawn.contains(&bid.listing));
        forgot |= row.bids.len() != before;
    }
    // Doc 19 cut A.4b: the emergency bids this retire withdrew, the same way.
    let before = map.emergencies.len();
    map.emergencies
        .retain(|_, bid| !withdrawn.contains(&bid.listing));
    forgot |= map.emergencies.len() != before;
    if forgot {
        put_port_map(ctx, map);
    }
}
