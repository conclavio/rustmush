//! **Occator's drones: a house's yard at a site** (doc 20 cut SM.12b; doc 20 §12.3 "Occator's
//! drones", §9.6, §8 "Leaving it running", D-20-11; the lead's rulings 1–8 of the SM.12b note,
//! doc 20a §4, the owner's rulings on O-8, O-9, O-10 and O-14 of 2026-10-24: "keep the
//! defaults … I'll tune later").
//!
//! **What the yard is.** A site's `site.yard` block ([`YardDef`], lane v15: Occator Rim) is laid up
//! by the applier as its **part (i)** (`lay_up`): a hull of the yard's
//! unit (the Dray-P "Occator Yard", `OWP-YD-01`) laid up by the very steps a programme hull is
//! (`crate::port::hull::lay_up_one` over `HullRows::Yard`), settled for good on the site's domain
//! at the pad's arrival pose `(0, 0)` (O-13) and titled to the house; its hold a vault of the house
//! at the org's root scope, backed by the hull's stowage; its bay holding the yard's rigs —
//! ordinary terrain vehicles of the house's own pool unit and tools
//! (`crate::port::vehicle::lay_up_rig` over `RigRows::Yard`), placed in the bay's rigs compartment
//! and titled to the house; and a `DARK` **dispatcher** Thing in the site's host room — the rigs'
//! operator and the house's acting principal for them. The map records it all on the site row
//! ([`crate::port::map::YardRow`]); nothing is submitted by the applier.
//!
//! **The house view** (the lead's ruling 2: "the lease machinery sees Occator as the rigs' own
//! lessor-operator — no lease"). Every operator gate SM.3b–SM.11 built reads a `TvLease`, so a
//! yard rig is read through a never-stored **house view** of that shape
//! (`crate::port::tv::TvLease::house_view`: its pilot the dispatcher, its hull the yard hull,
//! `HOUSE_LEASE` its lease word). Three one-reader seams carry it everywhere: **the hull's rig
//! census** (`crate::port::tv::live_on_hull` appends [`house_rigs`] for a yard hull), **the orphan
//! rule** (`crate::surface::plan::helm_hull` reads [`operating_hull`]) and **the lessee**
//! ([`rig_lease`] — the store's live record, else the house view). No reader in `surface/tick.rs`
//! or `port/tv.rs` is left on `TacsimStore::tv_lease_on` except `repossess` (stored-only by
//! design: a repossession ends stored leases, and refuses a yard hull outright) and the take's
//! stored-record read (a take is on a chartered programme hull, never the yard). The dispatcher
//! operates by these explicit branches, **never by a helm** (D2): `coop::helm_of` is untouched,
//! so every helm-gated verb refuses it, and `ship::no_charter` answers [`YARD_STAYS`]. For tenure
//! and the licence its operator principal is the house ([`operator_principal`], D3): SM.10's
//! seeded claims name `Organization(house)`, and the industry predicate asks the house's licence
//! account.
//!
//! **The seeds** (the lead's rulings 2–3; D5, D16, D26). The site clerk, after SM.10's held seeds
//! and before its paper seeds (`crate::port::site`'s `claims_step` items 7a, 8–9, 7b), records the
//! house's **standing licence** — an `Active` account of the house in the licence instrument, no
//! fee, a century ([`crate::port::map::HouseLicence`]; O-12) — then **surveys** each of the
//! house's held blocks into the dispatcher's own chart row at R1, rock footing, stamped `via:
//! Plan` and never evidence (`crate::surface::chart::write_seed`), one block a fire, through this
//! module's own action ([`DISPATCH_ACTION`], the chart family being written only in tacsim's
//! bundles). The clerk writes another principal's chart row and licence by ruling, on SM.10's
//! precedent of seeding the house's claims in the org's name; the alternative — the applier
//! seeding them (12b-E7) — was considered and not taken (the applier submits nothing). **The
//! papers never wait on the house** (`clerk_item`): an item that
//! cannot progress answers `None`, so SM.10's paper seed lands the same fire.
//!
//! **The delivery, and why it is not a sale** (the lead's ruling 4; D8, D9; O-10, O-11). The
//! dispatcher's own arm (`crate::port_tick`'s `Arm::Dispatch`,
//! `dispatch_step`) moves the yard hold's ore **in kind** to
//! the house's own storefront pool of that ore — the pool its mill (SM.12a) mills from — by one
//! receipted inventory `transfer` (debit, credit and the lineage carried) under the
//! [`DispatchLedger`] in the same bundle: no market call, no economy post, no contract, no marker.
//! It is bounded so the storefront holds at most one IC day of the mill's rate ([`deliverable`]):
//! the mill's rate, not the drones, sets the supply, and player ore sold into the house's bid lands
//! in the same pool and displaces the drones', never the reverse.
//!
//! **The filing and the re-arm** (O-14; D6, D7, D21, D27). Otherwise the dispatcher files one idle
//! rig's SM.11 fill plan (`F,G family=ice_ore`) through the very compiler and filing path `+tv/plan
//! … go` uses (`crate::surface::act::plan_ask`, `crate::surface::act::file_compiled`); the plan
//! then runs under the surface tick exactly as a player's. **The one difference from a player's is
//! the re-arm** (`file_due`): after a plan ends `HoldFull` or
//! `Bound` the next is filed once the yard's interval has passed; after `WorkedOut` only with fair
//! ore left that the house has read and a cycle landed; never after a staff clear, a right, a
//! stranding, a full chart or the dark registry. A rig whose last unload left ore in its hopper is
//! never re-filed into an immediate `HoldFull` end: it is skipped and named on the Dispatch
//! readiness line with its staff remedy, a hand unload as the dispatcher.
//!
//! **The restart argument.** Every decision is re-read from durable state on every fire — the
//! yard row, the rigs' records, the ledgers, the pools, the charts, the claims, the IC clock.
//! The RAM back-offs (`PortTickState`'s `dispatch_*` fields) only delay an act or keep a warning
//! to once a boot; a restart empties them and the next fire re-derives everything (a filing lost
//! with its RAM order is re-filed under the same key by the next eligible fire).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use mush_content::Pack;
use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalId, Ctx, DefinitionRef, IdempotencyKeyText, ObjId, PerceivedRef, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    RevisionSet, RulesValue, StateFamilyHandle, SubjectKey, World,
};
use mush_economy::{CustodyRef, EconomyStore, Vault, VaultId, VaultStatus};
use mush_inventory::InventoryFamilies;
use mush_inventory::record::EndpointRef;
use mush_org::{OrgId, OrgScopeId, PrincipalId, SystemPrincipalId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use smol_str::SmolStr;

use crate::definitions::{BayDef, InteriorRoomDef};
use crate::port::def::{BLOCK_LETTERS, MillDef, PortDef, SiteDef, Tenure, VehiclesDef, YardDef};
use crate::port::hull::{HullPlan, HullRows, REGISTRY_ISSUER, live, registry_mark};
use crate::port::map::{PortMap, PortMapState, SiteRow, YardRow};
use crate::port::readiness::LineState;
use crate::port::site_map::SurfaceReads;
use crate::port::tv::TvLease;
use crate::port::vehicle::{RigPlace, RigPlan, RigRows};
use crate::port_tick::{Fire, PortTickState, Step, port_error};
use crate::store::{TacsimStore, put_port_map};
use crate::surface::chart::{ChartKey, ChartRecord};
use crate::surface::claims::{BlockKey, BlockTenure};
use crate::surface::geology::{OreFamily, Word};
use crate::surface::plan::{FillPlan, FillState, PlanEnd};

// ---- the constants and the texts ---------------------------------------------------------

/// The dispatch action (doc 20 cut SM.12b): a site clerk's house survey, or a dispatcher's
/// delivery.
pub const DISPATCH_ACTION: &str = "tacsim_dispatch";
/// Its outcome: fields `act`, `actor`, `receipt`, `site`, `t` — never a reserve or a class.
pub const DISPATCH_OUTCOME: &str = "tacsim_dispatch_result";
/// The dispatch-ledger family: one [`DispatchLedger`] per yard, keyed by its site.
pub const DISPATCH_FAMILY: &str = "tacsim.dispatch";
/// The dispatch acts' idempotency-key prefix: `tacsim.dispatch/{site}/survey/{l}` (the letter
/// lower-cased) and `tacsim.dispatch/{site}/deliver/{n}`.
pub const DISPATCH_KEY_PREFIX: &str = "tacsim.dispatch/";
/// How many of its last deliveries a ledger keeps, its durable audit tail.
pub const MAX_RECENT_DELIVERIES: usize = 16;
/// How many times one delivery ordinal is submitted with the ledger not advancing before the
/// dispatcher passes its delivery over for the rest of the IC day (SM.12a's mill rule).
pub const MAX_DISPATCH_ATTEMPTS: u8 = 3;

/// **The dispatcher's answer to every ship verb** (doc 20 cut SM.12b, D2; O-18): `{name}` the
/// yard's name, `{org}` the house's name, `{site}` the site's name.
pub const YARD_STAYS: &str = "The {name} is {org}'s yard hull, settled for good on the pad at {site}; she never lifts and never trades.";
/// **A house rig's `+tv` slot line** (O-18): `{n}`, `{mark}`, `{where}`, `{charge}`, `{hopper}`.
pub const HOUSE_RIG_SLOT: &str = "Rig {n}: {mark}, {where}; charge {charge} %; hopper {hopper}.";
/// **The line after a house rig's slot line** (O-18): `{org}` the house's name.
pub const HOUSE_RIG_LINE: &str = "  {org}'s own rig, run by its dispatcher; no lease.";
/// **The dispatcher Thing's name** (O-8/O-18): `{name}` the yard's name ("Occator Yard
/// dispatcher"). Stored on a kept world's Thing.
pub const DISPATCHER_NAME: &str = "{name} dispatcher";
/// **The house licence account's name** (O-8/O-18): `{org}` the house's name ("Occator Water &
/// Propellant standing prospecting licence"). Stored on a kept world's account.
pub const HOUSE_LICENCE_NAME: &str = "{org} standing prospecting licence";

/// Dispatch line, order 1 (Missing): `{entry}`, `{org}`.
pub const DISPATCH_MISSING: &str = "{entry} dispatch {org}: the yard is not laid up.";
/// Order 2 (Flagged): `{what}` — `rig {n}'s record`, `rig {n}'s hopper`, `the delivery ledger`,
/// `the yard hold`, `the storefront`.
pub const DISPATCH_UNREADABLE: &str = "{entry} dispatch {org}: {what} cannot be read.";
/// Order 3 (Flagged; an unreachable guard — the instrument is a pinned constant).
pub const DISPATCH_NO_INSTRUMENT: &str =
    "{entry} dispatch {org}: no licence instrument is set, so the house cannot be licensed.";
/// Order 4 (Flagged).
pub const DISPATCH_LICENCE_INACTIVE: &str =
    "{entry} dispatch {org}: its licence account is not active.";
/// Order 5a (Flagged): `{n}`, `{mark}` — while the rig is out of the bay; once staff have brought
/// it in, its stranded plan reads order 5b (clear it to start the rig again).
pub const DISPATCH_STRANDED: &str = "{entry} dispatch {org}: rig {n} ({mark}) is stranded on the ground. Staff bring it in by placing it on the pad, then recover it as the dispatcher.";
/// Order 5b (Flagged): `{n}`, `{mark}`, `{why}` (`PlanEnd::word`; "stranded" for a stranded rig
/// back in the bay).
pub const DISPATCH_STOPPED: &str = "{entry} dispatch {org}: rig {n} ({mark}) stopped: {why}. Staff clear its plan to start it again.";
/// Order 5c (Flagged): `{n}`, `{mark}`.
pub const DISPATCH_HELD: &str =
    "{entry} dispatch {org}: rig {n} ({mark}) is held by staff; strike its plan to start it again.";
/// Order 5d (Flagged): `{n}`, `{mark}`.
pub const DISPATCH_STALLED: &str = "{entry} dispatch {org}: rig {n} ({mark}) holds ore its last unload left in the hopper. Staff unload it as the dispatcher (+tv/{n} unload go) to start it again.";
/// Order 6 (Ready): `{rigs}`, `{hull}`.
pub const DISPATCH_NOT_OPEN: &str = "{entry} dispatch {org}: {rigs} in the {hull}'s bay; the site is not open, so nothing is filed.";
/// Order 7 (Pending).
pub const DISPATCH_OWED: &str =
    "{entry} dispatch {org}: waiting on the Registry's seeds, licence and survey.";
/// Order 8 (Ready): `{ore}`, `{rigs}`, `{hull}`, `{total_t}`.
pub const DISPATCH_WORKED_OUT: &str = "{entry} dispatch {org}: its blocks read worked out in {ore}; {rigs} parked in the {hull}'s bay, {total_t} t delivered in all.";
/// Order 9 (Ready): `{out}`, `{rigs}`, `{working}`, `{filed}`, `{day_t}`, `{ore}`, `{total_t}`.
pub const DISPATCH_WORKING: &str = "{entry} dispatch {org}: {out} of {rigs} out, {working} working a plan, {filed} filed in all; {day_t} t of {ore} delivered to its storefront today, {total_t} t in all.";
/// Order 10 (Ready): `{rigs}`, `{hull}`.
pub const DISPATCH_NOTHING: &str =
    "{entry} dispatch {org}: {rigs} in the {hull}'s bay; nothing filed yet.";

// ---- the yard and its queries -------------------------------------------------------------

/// **One latched lane site's yard, resolved** (doc 20 cut SM.12b).
#[derive(Clone, Debug)]
pub struct YardView<'w> {
    /// The site's port entry.
    pub entry: CanonicalId,
    /// The site block.
    pub site: &'w SiteDef,
    /// The yard block.
    pub def: &'w YardDef,
    /// The site's port map.
    pub map: &'w PortMap,
    /// The yard's map row.
    pub row: &'w YardRow,
    /// The house: `crate::port::site::org_principal` of the yard's org.
    pub house: PrincipalId,
}

impl YardView<'_> {
    /// The yard hull (its one row), once laid up.
    #[must_use]
    pub fn hull(&self) -> Option<ObjId> {
        self.row.hulls.first().map(|row| ObjId(row.hull))
    }

    /// The dispatcher, once laid up.
    #[must_use]
    pub fn dispatcher(&self) -> Option<ObjId> {
        self.row.dispatcher.map(ObjId)
    }
}

/// Every latched lane port map's yard row, entry order — the cheap scan the hot seams read
/// before resolving anything (no pack read).
fn yard_rows(w: &World) -> Vec<(&PortMap, &YardRow)> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    store
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .filter_map(|map| Some((map, map.site.as_ref()?.yard.as_ref()?)))
        .filter(|(map, _)| crate::port::stock::entry_latched(w, &map.entry))
        .collect()
}

/// **Every latched lane site's yard** (doc 20 cut SM.12b): each latched lane port map whose site
/// row records a yard, its site and yard blocks from the report's lane pack
/// (`crate::port::readiness::report_ports`, the `clerk_site` precedent), its house resolved —
/// entry order. A yard whose house has no lane org row is left out (nothing it does could act).
#[must_use]
pub fn yards(w: &World) -> Vec<YardView<'_>> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    if yard_rows(w).is_empty() {
        return Vec::new();
    }
    let Ok(lane) = CanonicalId::new(crate::definitions::content::LANE_PACK) else {
        return Vec::new();
    };
    let (_, ports) = crate::port::readiness::report_ports(store, w);
    let mut out = Vec::new();
    for (entry, def) in ports {
        let Some(site) = def.site.as_ref() else {
            continue;
        };
        let Some(yard) = site.yard.as_deref() else {
            continue;
        };
        if !crate::port::stock::entry_latched(w, &entry) {
            continue;
        }
        let PortMapState::Present(map) = store.port_map(&lane, &entry) else {
            continue;
        };
        let Some(row) = map.site.as_ref().and_then(|row| row.yard.as_ref()) else {
            continue;
        };
        let Some(house) = crate::port::site::org_principal(w, &yard.org) else {
            continue;
        };
        out.push(YardView {
            entry,
            site,
            def: yard,
            map,
            row,
            house,
        });
    }
    out
}

/// **The yard `actor` dispatches** (doc 20 cut SM.12b): the yard whose row records it as its
/// dispatcher.
#[must_use]
pub fn yard_of_dispatcher(w: &World, actor: ObjId) -> Option<YardView<'_>> {
    if !yard_rows(w)
        .iter()
        .any(|(_, row)| row.dispatcher == Some(actor.0))
    {
        return None;
    }
    yards(w)
        .into_iter()
        .find(|yard| yard.row.dispatcher == Some(actor.0))
}

/// **The yard `hull` is the hull of** (doc 20 cut SM.12b): the yard whose row names it.
#[must_use]
pub fn yard_of_hull(w: &World, hull: ObjId) -> Option<YardView<'_>> {
    if !yard_rows(w)
        .iter()
        .any(|(_, row)| row.hulls.iter().any(|r| r.hull == hull.0))
    {
        return None;
    }
    yards(w)
        .into_iter()
        .find(|yard| yard.row.hulls.iter().any(|r| r.hull == hull.0))
}

/// **Whether `person` is the dispatcher of the yard whose hull is `hull`** (doc 20 cut SM.12b):
/// `voyage::aboard`'s one branch — the dispatcher is aboard its own yard hull wherever it
/// stands, and nowhere else.
#[must_use]
pub fn is_dispatcher_of(w: &World, hull: ObjId, person: ObjId) -> bool {
    yard_rows(w).iter().any(|(_, row)| {
        row.dispatcher == Some(person.0) && row.hulls.iter().any(|r| r.hull == hull.0)
    })
}

/// The live dispatcher's yard hull, by the cheap scan.
fn dispatched_hull(w: &World, actor: ObjId) -> Option<ObjId> {
    if crate::port_tick::gone(w, actor) {
        return None;
    }
    yard_rows(w)
        .into_iter()
        .find(|(_, row)| row.dispatcher == Some(actor.0))
        .and_then(|(_, row)| row.hulls.first().map(|r| ObjId(r.hull)))
}

/// **The hull `actor` operates** (doc 20 cut SM.12b; the orphan rule's seam): its helm's hull
/// (`crate::port::coop::helm_of`), else — a live dispatcher's — its yard hull.
#[must_use]
pub fn operating_hull(w: &World, actor: ObjId) -> Option<ObjId> {
    crate::port::coop::helm_of(w, actor)
        .map(|helm| helm.hull)
        .or_else(|| dispatched_hull(w, actor))
}

/// **Whether `actor` operates `hull`** ([`operating_hull`]).
#[must_use]
pub fn operates(w: &World, actor: ObjId, hull: ObjId) -> bool {
    operating_hull(w, actor) == Some(hull)
}

/// **The principal `actor` operates for** (doc 20 cut SM.12b, D3): the house for a yard's
/// dispatcher (its seeded claims and its licence are the house's), else `Object(actor)`.
#[must_use]
pub fn operator_principal(w: &World, actor: ObjId) -> PrincipalId {
    yard_of_dispatcher(w, actor).map_or(PrincipalId::Object(actor), |yard| yard.house)
}

/// The house views of one yard row's rigs, slot order (`slot = n + 1`).
fn house_views(row: &YardRow, dispatcher: ObjId) -> Vec<TvLease> {
    let Some(hull) = row.hulls.first().map(|r| ObjId(r.hull)) else {
        return Vec::new();
    };
    let mut views: Vec<TvLease> = row
        .rigs
        .iter()
        .filter_map(|rig| {
            let slot = u8::try_from(rig.n.checked_add(1)?).ok()?;
            Some(TvLease::house_view(
                ObjId(rig.tv),
                &rig.mark,
                dispatcher,
                hull,
                slot,
            ))
        })
        .collect();
    views.sort_by_key(|view| view.slot);
    views
}

/// **The yard's rigs as house views** (doc 20 cut SM.12b; B5): for the yard `dispatcher`
/// dispatches, each rig row as `TvLease::house_view` — the dispatcher its pilot, the yard hull
/// its hull, its row's mark, slot `n + 1` — slot order. Empty for anyone else.
#[must_use]
pub fn house_rigs(w: &World, dispatcher: ObjId) -> Vec<TvLease> {
    yard_rows(w)
        .into_iter()
        .find(|(_, row)| row.dispatcher == Some(dispatcher.0))
        .map_or_else(Vec::new, |(_, row)| house_views(row, dispatcher))
}

/// **A rig's lessee record** (doc 20 cut SM.12b; **the one lessee reader**, B5 item 19): the
/// store's live record (`TacsimStore::tv_lease_on`, cloned), else — a yard rig — its house
/// view. `None` for a rig neither leased nor a yard's.
#[must_use]
pub fn rig_lease(w: &World, tv: ObjId) -> Option<TvLease> {
    if let Some(record) = w
        .module_state::<TacsimStore>()
        .and_then(|store| store.tv_lease_on(tv))
    {
        return Some(record.clone());
    }
    yard_rows(w).into_iter().find_map(|(_, row)| {
        if !row.rigs.iter().any(|rig| rig.tv == tv.0) {
            return None;
        }
        let dispatcher = ObjId(row.dispatcher?);
        house_views(row, dispatcher)
            .into_iter()
            .find(|view| view.tv == tv)
    })
}

/// **Whether `tv` is a yard's rig** (doc 20 cut SM.12b), by the cheap scan — the plan counters'
/// filter: the house's plans run in every keyed world, and the counters count the players' only.
#[must_use]
pub(crate) fn is_yard_rig(w: &World, tv: ObjId) -> bool {
    yard_rows(w)
        .iter()
        .any(|(_, row)| row.rigs.iter().any(|rig| rig.tv == tv.0))
}

/// **Every yard's live dispatcher** (the port tick's `Arm::Dispatch` agents), entry order.
#[must_use]
pub fn dispatchers(w: &World) -> Vec<ObjId> {
    yards(w)
        .iter()
        .filter_map(YardView::dispatcher)
        .filter(|dispatcher| !crate::port_tick::gone(w, *dispatcher))
        .collect()
}

/// **The site's blocks held by `org`**, letter order (F, G, J, K at Occator Rim): what the house
/// surveyed and seeded — all of them, not only the yard's worked `blocks`.
#[must_use]
pub(crate) fn held_letters(site: &SiteDef, org: &CanonicalId) -> Vec<char> {
    BLOCK_LETTERS
        .into_iter()
        .filter(|letter| {
            let terms = site.block(*letter);
            terms.tenure == Tenure::Held && terms.holder == Some(org)
        })
        .collect()
}

/// An org's authored name in the report's lane pack (its id when not found).
fn org_name(w: &World, org: &CanonicalId) -> String {
    crate::port::site::org_name_of(w, org)
}

/// **`YARD_STAYS` for `actor`** (doc 20 cut SM.12b): `Some` filled for a yard's dispatcher.
#[must_use]
pub(crate) fn yard_stays_line(w: &World, actor: ObjId) -> Option<String> {
    let yard = yard_of_dispatcher(w, actor)?;
    Some(
        YARD_STAYS
            .replace("{name}", &yard.def.name)
            .replace("{org}", &org_name(w, &yard.def.org))
            .replace("{site}", &yard.site.name),
    )
}

/// **`HOUSE_RIG_LINE` for the yard `dispatcher` dispatches** (doc 20 cut SM.12b).
#[must_use]
pub(crate) fn house_rig_line(w: &World, dispatcher: ObjId) -> String {
    let org = yard_of_dispatcher(w, dispatcher)
        .map_or_else(String::new, |yard| org_name(w, &yard.def.org));
    HOUSE_RIG_LINE.replace("{org}", &org)
}

// ---- the lay-up (the applier's part (i)) ---------------------------------------------------

/// **What the yard's lay-up reads of the whole pack** (doc 20 cut SM.12b): the pack's one rig
/// pool (the yard's rigs are of its unit, tools and issuer) and every org a port entry of the
/// pack authors (a house org another entry of the same apply makes before this one).
#[derive(Clone, Debug, Default)]
pub(crate) struct YardPack {
    /// The pack's one `vehicles` block.
    pub pool: Option<VehiclesDef>,
    /// Every org id a port entry authors.
    pub orgs: Vec<CanonicalId>,
}

/// [`YardPack`] from the pack being applied (port entries that do not decode are skipped: the
/// pack's own rules refuse them first).
pub(crate) fn pack_facts(pack: &Pack) -> YardPack {
    let mut facts = YardPack::default();
    let Ok(kind) = CanonicalId::new("port") else {
        return facts;
    };
    for (_, entry) in pack.entries_of(&kind) {
        let Ok(def) = entry.decode::<PortDef>() else {
            continue;
        };
        if facts.pool.is_none() {
            facts.pool = def.vehicles.as_deref().cloned();
        }
        facts.orgs.extend(def.orgs.iter().map(|org| org.id.clone()));
    }
    facts
}

/// The yard unit's bays and their room templates (the unit's own, never a refit's).
fn unit_bays(interior: &crate::definitions::InteriorDef) -> Vec<(BayDef, InteriorRoomDef)> {
    interior
        .bays
        .iter()
        .filter_map(|bay| Some((bay.clone(), interior.room(&bay.room)?.clone())))
        .collect()
}

/// **Every yard refusal decidable before a write** (doc 20 cut SM.12b; B4 item 14), each
/// prefixed `"{id}: "`: a recorded yard with no authored yard; the unit not fielded, without
/// interior or logistics, without a bay of at least `rigs`, or with no stowage compartment
/// accepting the ore's stowage tag; the pool's template (`crate::port::vehicle::template`'s own
/// sentences) and issuer; the house's org row absent from every lane port map and from the pack
/// being applied; a recorded hull, rig, dispatcher or hold vault gone; a recorded mark not the
/// authored prefix's. Answers `(hulls, rigs)` still to title, which the apply adds to its
/// frozen-economy counts.
///
/// # Errors
///
/// The first refusal, as its sentence.
#[allow(clippy::too_many_lines)] // one ordered check list
pub(crate) fn preflight(
    w: &World,
    source: &Pack,
    id: &CanonicalId,
    def: &PortDef,
    map: Option<&PortMap>,
) -> Result<(usize, usize), String> {
    let pack = source.manifest().id();
    let key = crate::port::port_map_key(pack, id);
    let recorded = map
        .and_then(|map| map.site.as_ref())
        .and_then(|site| site.yard.as_ref());
    let Some(yard) = def.site.as_ref().and_then(|site| site.yard.as_deref()) else {
        if recorded.is_some() {
            return Err(format!(
                "{id}: the port map {key} records a yard but the site has none; a yard is never \
                 retired by a port update"
            ));
        }
        return Ok((0, 0));
    };
    let name = &yard.name;
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| format!("{id}: the TacSim layer is not composed on this world"))?;
    let economy = w
        .module_state::<EconomyStore>()
        .ok_or_else(|| format!("{id}: the economy is not composed on this world"))?;
    // The unit: fielded, an interior and logistics, a bay holding `rigs`, an ore hold.
    let unfit = || {
        format!(
            "{id}: yard {name}: unit {} cannot carry {} rigs and an ore hold",
            yard.unit, yard.rigs
        )
    };
    let (interior, logistics) = crate::port::hull::spec_for(store, &yard.unit).ok_or_else(unfit)?;
    if !store.definitions().bindings.contains_key(&yard.unit) {
        return Err(unfit());
    }
    let bay_fits = interior
        .bay()
        .is_some_and(|bay| bay.slot.maximum_items >= yard.rigs);
    let ore_tag = store
        .goods()
        .and_then(|goods| goods.good(&yard.ore))
        .map(|good| good.stowage.clone());
    let holds_ore = ore_tag.is_some_and(|tag| {
        logistics
            .stowage
            .values()
            .any(|compartment| compartment.accepts.contains(&tag))
    });
    if !bay_fits || !holds_ore {
        return Err(unfit());
    }
    crate::port::hull::berths_slot_of(&format!("yard {name}"), &yard.unit, interior)
        .map_err(|e| format!("{id}: {e}"))?;
    // The pool: its unit and its issuer.
    let facts = pack_facts(source);
    let pool = facts
        .pool
        .as_ref()
        .filter(|pool| pool.id == yard.pool)
        .ok_or_else(|| {
            format!(
                "{id}: site: yard: pool {} is not the pack's rig pool of {}",
                yard.pool, yard.org
            )
        })?;
    crate::port::vehicle::template(store, pool).map_err(|e| format!("{id}: {e}"))?;
    for issuer in [REGISTRY_ISSUER, pool.issuer.as_str()] {
        let registered =
            SystemPrincipalId::new(issuer).is_some_and(|issuer| economy.is_title_issuer(&issuer));
        if !registered {
            return Err(format!(
                "{id}: yard {name}: the title issuer {issuer} is not registered on this world"
            ));
        }
    }
    // The house: an org row on some lane map, or an org this very apply makes.
    let on_map = store
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .any(|map| map.orgs.contains_key(&yard.org));
    if !on_map && !facts.orgs.contains(&yard.org) {
        return Err(format!(
            "{id}: yard {name}: {} has no map row yet",
            yard.org
        ));
    }
    // The recorded objects: never re-created, never re-marked.
    let mut to_title = (0_usize, 0_usize);
    let Some(row) = recorded else {
        return Ok((1, usize::try_from(yard.rigs).unwrap_or(usize::MAX)));
    };
    let gone = |what: &str, n: u64| {
        format!("{id}: yard {name}: the recorded {what} #{n} is gone; it is never re-created")
    };
    let wrong_mark = |mark: &str, want: &str| {
        format!(
            "{id}: yard {name}: the recorded mark {mark} is not {want}; a mark is never re-issued"
        )
    };
    for hull in &row.hulls {
        let want = registry_mark(&yard.registry_prefix, hull.n);
        if hull.mark != want {
            return Err(wrong_mark(&hull.mark, &want));
        }
        let objects = std::iter::once(hull.hull)
            .chain(hull.rooms.values().copied())
            .chain(hull.exits.iter().copied());
        for obj in objects {
            if !live(w, obj) {
                return Err(gone("hull", u64::from(obj)));
            }
        }
        if economy.snapshot().asset(ObjId(hull.hull)).is_none() {
            to_title.0 = to_title.0.saturating_add(1);
        }
    }
    if row.hulls.is_empty() {
        to_title.0 = 1;
    }
    for rig in &row.rigs {
        let want = registry_mark(&yard.rig_prefix, rig.n);
        if rig.mark != want {
            return Err(wrong_mark(&rig.mark, &want));
        }
        for obj in std::iter::once(rig.tv).chain(rig.rooms.values().copied()) {
            if !live(w, obj) {
                return Err(gone("rig", u64::from(obj)));
            }
        }
        if economy.snapshot().asset(ObjId(rig.tv)).is_none() {
            to_title.1 = to_title.1.saturating_add(1);
        }
    }
    let absent = (0..yard.rigs)
        .filter(|n| !row.rigs.iter().any(|rig| rig.n == *n))
        .count();
    to_title.1 = to_title.1.saturating_add(absent);
    if let Some(dispatcher) = row.dispatcher
        && !live(w, dispatcher)
    {
        return Err(gone("dispatcher", u64::from(dispatcher)));
    }
    if let Some(hold) = row.hold {
        let house = crate::port::site::org_principal(w, &yard.org);
        let ours = economy
            .snapshot()
            .vault(VaultId(hold))
            .is_some_and(|vault| {
                Some(&vault.owner) == house.as_ref() && vault.status == VaultStatus::Active
            });
        if !ours {
            return Err(gone("hold vault", hold));
        }
    }
    Ok(to_title)
}

/// **Part (i): lay up a site's yard** (doc 20 cut SM.12b; B4 item 15). In order, each step
/// idempotent, the map written after every object: (i1) the hull — the yard unit's template
/// through `crate::port::hull::lay_up_one` over `HullRows::Yard` (`n = 0`, the yard's name and
/// mark, its own unit's bays), placed at `(0, 0)` on the site's domain and titled to the house
/// under `REGISTRY_ISSUER`; (i2) the hold vault, the house's at its root scope, backed by the
/// hull's stowage; (i3) the rigs, `n = 0..rigs`, of the pool's unit and template, created and
/// placed in the yard hull's bay room in its rigs compartment, titled to the house under the
/// pool's issuer; (i4) the dispatcher, a `DARK` Thing named [`DISPATCHER_NAME`] in the host
/// room. **Nothing is submitted**; a re-apply creates nothing.
///
/// # Errors
///
/// A refusal no write in this pass could have foreseen, named.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // one ordered lay-up
pub(crate) fn lay_up(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    pool: &VehiclesDef,
    yard: &YardDef,
    map: &mut PortMap,
    domain: mush_sim::data::DomainId,
    gallery: ObjId,
    host: ObjId,
) -> Result<(), String> {
    let name = &yard.name;
    let label = format!("yard {name}");
    let (interior, spec, rig_interior, rig_spec) = {
        let store = ctx
            .w
            .module_state::<TacsimStore>()
            .ok_or_else(|| format!("{id}: the TacSim layer is not composed"))?;
        let (interior, spec) = crate::port::hull::template_of(store, &label, &yard.unit)
            .map_err(|e| format!("{id}: {e}"))?;
        let (rig_interior, rig_spec, _) =
            crate::port::vehicle::template(store, pool).map_err(|e| format!("{id}: {e}"))?;
        (
            interior.clone(),
            spec.clone(),
            rig_interior.clone(),
            rig_spec.clone(),
        )
    };
    let slot = crate::port::hull::berths_slot_of(&label, &yard.unit, &interior)
        .map_err(|e| format!("{id}: {e}"))?;
    let bays = unit_bays(&interior);
    let house_row = crate::port::site::map_org(ctx.w, &yard.org)
        .ok_or_else(|| format!("{id}: yard {name}: {} has no map row yet", yard.org))?;
    let registry = SystemPrincipalId::new(REGISTRY_ISSUER)
        .ok_or_else(|| format!("{id}: {REGISTRY_ISSUER} is not a system principal"))?;
    // (i1) The hull.
    crate::port::hull::lay_up_one(
        ctx,
        id,
        map,
        &HullPlan {
            name: Some(name.as_str()),
            mark: registry_mark(&yard.registry_prefix, 0),
            rows: HullRows::Yard,
            interior: &interior,
            spec: &spec,
            slot: &slot,
            bays: &bays,
            n: 0,
        },
        gallery,
    )?;
    let hull_row = crate::port::hull::rows(map, HullRows::Yard)
        .first()
        .cloned()
        .ok_or_else(|| format!("{id}: yard {name}: the hull has no map row"))?;
    let hull = ObjId(hull_row.hull);
    crate::port::hull::place_and_title(ctx, id, hull, (0, 0), domain, &registry, &house_row)?;
    // (i2) The hold vault.
    let stowage = hull_row
        .rooms
        .get(&interior.store)
        .map(|room| ObjId(*room))
        .ok_or_else(|| format!("{id}: yard {name}: the hull has no stowage"))?;
    if yard_row(map).is_none_or(|row| row.hold.is_none()) {
        let vault = crate::port::apply::alloc_vault_id(ctx)
            .map_err(|e| format!("{id}: yard {name}: the hold vault: {e}"))?;
        mush_economy::create_vault(
            ctx,
            Vault {
                id: vault,
                owner: PrincipalId::Organization(OrgId(house_row.org)),
                scope: Some(OrgScopeId(house_row.root_scope)),
                name: SmolStr::new(format!("{name} hold")),
                backing: CustodyRef::SpatialInterior(stowage),
                status: VaultStatus::Active,
                revision: 1,
            },
        );
        yard_row_mut(map).hold = Some(vault.0);
        put_port_map(ctx, map);
    }
    // (i3) The rigs, in the bay's rigs compartment.
    let (bay, bay_room) = interior
        .bay()
        .and_then(|bay| Some((bay, ObjId(*hull_row.rooms.get(&bay.room)?))))
        .ok_or_else(|| format!("{id}: yard {name}: the hull's bay is not laid up"))?;
    let bay_slot = mush_sim::SlotId::new(bay.slot.id.as_str())
        .ok_or_else(|| format!("{id}: yard {name}: the bay slot is not a slot id"))?;
    let rig_slot =
        mush_sim::SlotId::new(rig_interior.berths_slot.id.as_str()).ok_or_else(|| {
            format!(
                "{id}: vehicles {}: unit {}'s berths compartment '{}' is not a simulation slot id",
                pool.id, pool.unit, rig_interior.berths_slot.id
            )
        })?;
    let pool_issuer = SystemPrincipalId::new(&pool.issuer).ok_or_else(|| {
        format!(
            "{id}: vehicles {}: the title issuer {} is not a system principal",
            pool.id, pool.issuer
        )
    })?;
    let place = RigPlace::Bay {
        room: bay_room,
        slot: bay_slot,
    };
    for n in 0..yard.rigs {
        crate::port::vehicle::lay_up_rig(
            ctx,
            id,
            map,
            &RigPlan {
                mark: registry_mark(&yard.rig_prefix, n),
                rows: RigRows::Yard,
                place: place.clone(),
                interior: &rig_interior,
                spec: &rig_spec,
                slot: &rig_slot,
                n,
            },
        )?;
        let tv = crate::port::vehicle::rig_index(map, RigRows::Yard, n)
            .and_then(|i| crate::port::vehicle::rig_rows(map, RigRows::Yard).get(i))
            .map(|rig| ObjId(rig.tv))
            .ok_or_else(|| format!("{id}: yard {name}: rig {n} has no map row"))?;
        crate::port::vehicle::place_and_title(ctx, id, tv, &place, &pool_issuer, &house_row)?;
    }
    // (i4) The dispatcher.
    if yard_row(map).is_none_or(|row| row.dispatcher.is_none()) {
        let thing =
            crate::port::site::dark_thing(ctx, id, &DISPATCHER_NAME.replace("{name}", name), host)?;
        yard_row_mut(map).dispatcher = Some(thing);
        put_port_map(ctx, map);
    }
    Ok(())
}

/// The map's yard row.
fn yard_row(map: &PortMap) -> Option<&YardRow> {
    map.site.as_ref().and_then(|site| site.yard.as_ref())
}

/// The map's yard row, made when absent.
fn yard_row_mut(map: &mut PortMap) -> &mut YardRow {
    map.site
        .get_or_insert_with(SiteRow::default)
        .yard
        .get_or_insert_with(YardRow::default)
}

// ---- the dispatch family -----------------------------------------------------------------

/// **The dispatch ledger's key** (doc 20 cut SM.12b): the yard's site entry; its key text is the
/// site id, parsed strictly.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DispatchKey {
    /// The yard's site entry.
    pub site: CanonicalId,
}

impl DispatchKey {
    /// The site id.
    #[must_use]
    pub fn key_text(&self) -> String {
        self.site.to_string()
    }

    /// The strict inverse of [`DispatchKey::key_text`]: a canonical id spelled exactly.
    #[must_use]
    pub fn parse(text: &str) -> Option<DispatchKey> {
        let site = CanonicalId::new(text).ok()?;
        (site.as_str() == text).then_some(DispatchKey { site })
    }
}

/// **One delivery, as its ledger's audit tail keeps it** (doc 20 cut SM.12b).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    /// The delivery's ordinal, from 1.
    pub ordinal: u64,
    /// The IC instant it was made at.
    pub at_ic: i64,
    /// Tonnes delivered, at least 1.
    pub t: u64,
}

/// **One yard's durable delivery receipt** (doc 20 cut SM.12b; the lead's ruling 4): the
/// totals, the IC day's tally, the last sixteen deliveries; the ordinal is the replay guard.
/// Replaced in every delivery's own bundle by `tacsim_dispatch`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchLedger {
    /// The yard's site entry (its key).
    pub site: CanonicalId,
    /// The house, `OrgId.0`.
    pub house: u64,
    /// The dispatcher the tick acted as.
    pub dispatcher: ObjId,
    /// The ore delivered.
    pub good: CanonicalId,
    /// The yard hold's vault.
    pub hold: VaultId,
    /// The house storefront's vault.
    pub storefront: VaultId,
    /// Deliveries made, `>= 1`.
    pub deliveries: u64,
    /// Tonnes delivered in all.
    pub delivered_t: u64,
    /// The IC day of the last delivery.
    pub day: i64,
    /// Tonnes delivered on `day`.
    pub day_t: u64,
    /// The last deliveries, oldest first, at most [`MAX_RECENT_DELIVERIES`].
    pub recent: Vec<Delivery>,
}

impl DispatchLedger {
    /// The ledger's invariants: at least one delivery; a tail of 1 to [`MAX_RECENT_DELIVERIES`]
    /// deliveries whose ordinals ascend by one to `deliveries`, each at least a tonne, summing to
    /// no more than the total; the day the last delivery's; the day's tally at least the last
    /// delivery and within the total.
    ///
    /// # Errors
    ///
    /// A message naming the first invariant the ledger breaks.
    pub fn check(&self) -> Result<(), String> {
        if self.deliveries == 0 {
            return Err("a dispatch ledger records at least one delivery".to_owned());
        }
        let Some(last) = self.recent.last() else {
            return Err("a dispatch ledger keeps its last delivery".to_owned());
        };
        if self.recent.len() > MAX_RECENT_DELIVERIES {
            return Err(format!(
                "a dispatch ledger keeps at most {MAX_RECENT_DELIVERIES} recent deliveries"
            ));
        }
        if last.ordinal != self.deliveries
            || self.recent.first().is_none_or(|first| first.ordinal == 0)
            || self.recent.windows(2).any(|pair| match pair {
                [a, b] => a.ordinal.checked_add(1) != Some(b.ordinal),
                _ => true,
            })
        {
            return Err(
                "a dispatch ledger's recent ordinals ascend by one to its delivery count"
                    .to_owned(),
            );
        }
        let mut tail: u64 = 0;
        for recent in &self.recent {
            if recent.t == 0 {
                return Err(format!(
                    "a dispatch ledger's delivery {} moves at least a tonne",
                    recent.ordinal
                ));
            }
            tail = tail
                .checked_add(recent.t)
                .ok_or_else(|| "a dispatch ledger's recent deliveries overflow".to_owned())?;
        }
        if tail > self.delivered_t {
            return Err("a dispatch ledger's recent deliveries sum within its total".to_owned());
        }
        if self.day != crate::port::mill::ic_day(last.at_ic) {
            return Err("a dispatch ledger's day is its last delivery's".to_owned());
        }
        if self.day_t < last.t || self.day_t > self.delivered_t {
            return Err(
                "a dispatch ledger's day tally is at least its last delivery and within its total"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

/// Set once the first key mismatch has been logged (the rule logs once per process).
static KEY_MISMATCH_LOGGED: AtomicBool = AtomicBool::new(false);

/// **One dispatch ledger, read through this module's own typed-state lens**, with the key
/// re-checked (`ledger.site == key.site`; `crate::port::mill`'s `ledger_at` rule). `Err` when the
/// lens refuses or the ledger names another site — fail closed, never "absent"; a mismatch is
/// logged once a process.
pub(crate) fn dispatch_at(
    ctx: &Ctx<'_>,
    handle: &StateFamilyHandle<DispatchKey, DispatchLedger>,
    key: &DispatchKey,
) -> Result<Option<DispatchLedger>, ()> {
    match ctx.action_state(handle, key).map_err(|_| ())? {
        None => Ok(None),
        Some(ledger) if ledger.site == key.site => Ok(Some(ledger.clone())),
        Some(_) => {
            if !KEY_MISMATCH_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::error!(
                    key = %key.key_text(),
                    "tacsim: a dispatch ledger is stored under another site's key; it reads as \
                     unreadable (fail closed)"
                );
            }
            Err(())
        }
    }
}

/// **The tonnes a delivery moves** (doc 20 cut SM.12b; the lead's ruling 4, O-10), pure: what the
/// yard hold holds, at most what tops the storefront up to `bound_t` — the house mill's one IC
/// day of ore (`MillDef::per_ic_day_t`), so the mill's rate, not the drones, sets the supply, and
/// player ore in the same pool displaces the drones'. Saturating.
#[must_use]
pub fn deliverable(hold_t: u64, front_t: u64, bound_t: u64) -> u64 {
    hold_t.min(bound_t.saturating_sub(front_t))
}

/// **The house's mill of the yard's ore** (doc 20 cut SM.12b): the storefront vault of the
/// house's vendor whose mill row takes `yard.def.ore` (the validator guarantees exactly one), and
/// that mill row. `None` when the vendor has no map row yet.
#[must_use]
pub(crate) fn house_mill<'w>(w: &'w World, yard: &YardView<'w>) -> Option<(VaultId, &'w MillDef)> {
    let store = w.module_state::<TacsimStore>()?;
    let lane = CanonicalId::new(crate::definitions::content::LANE_PACK).ok()?;
    let (_, ports) = crate::port::readiness::report_ports(store, w);
    for (entry, def) in ports {
        for vendor in &def.vendors {
            if vendor.org != yard.def.org {
                continue;
            }
            let Some(mill) = vendor.mill.iter().find(|mill| mill.ore == yard.def.ore) else {
                continue;
            };
            let PortMapState::Present(map) = store.port_map(&lane, &entry) else {
                return None;
            };
            let row = map.vendors.get(&vendor.org)?;
            return Some((VaultId(row.vault), mill));
        }
    }
    None
}

// ---- the provider -------------------------------------------------------------------------

/// **`tacsim_dispatch`** (doc 20 cut SM.12b; B8): two acts on one action, offered by who asks —
/// to a site clerk of a site with a yard, **`survey`** (one held block's four fields written
/// into the dispatcher's chart row at R1 rock, `via: Plan`; one write); to a yard's dispatcher,
/// **`deliver`** (the yard hold's ore moved to the house storefront by one receipted `transfer`
/// under the dispatch ledger, the ordinal its replay guard). No market call, no economy post, no
/// marker (D8).
pub(crate) struct DispatchProvider {
    /// The action.
    pub action: DefinitionRef,
    /// The outcome.
    pub outcome: DefinitionRef,
    /// `tacsim.dispatch`.
    pub dispatch: StateFamilyHandle<DispatchKey, DispatchLedger>,
    /// `tacsim.chart`.
    pub charts: StateFamilyHandle<ChartKey, ChartRecord>,
    /// `tacsim.claims`.
    pub claims: StateFamilyHandle<BlockKey, BlockTenure>,
    /// The delegated inventory families.
    pub inventory: InventoryFamilies,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "a dispatch act is self-scoped, takes no targets and names its row or its next delivery",
    )
}

/// The invocation's parameters, checked for shape.
fn parameters(
    invocation: &ActionInvocation,
) -> Result<&Map<String, Value>, mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let parameters = invocation.parameters().as_object().ok_or_else(invalid)?;
    match parameters.get("act").and_then(Value::as_str) {
        Some("survey") => {
            parameters
                .get("row")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
        }
        Some("deliver") => {
            if parameters
                .get("ordinal")
                .and_then(Value::as_u64)
                .is_none_or(|ordinal| ordinal == 0)
            {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    Ok(parameters)
}

fn text<'p>(row: &'p Map<String, Value>, key: &str) -> Result<&'p str, mush_core::ActionError> {
    row.get(key).and_then(Value::as_str).ok_or_else(invalid)
}

fn number(row: &Map<String, Value>, key: &str) -> Result<u64, mush_core::ActionError> {
    row.get(key).and_then(Value::as_u64).ok_or_else(invalid)
}

fn canonical(row: &Map<String, Value>, key: &str) -> Result<CanonicalId, mush_core::ActionError> {
    CanonicalId::new(text(row, key)?).map_err(|_| invalid())
}

fn object_of(value: u64) -> Result<ObjId, mush_core::ActionError> {
    u32::try_from(value).map(ObjId).map_err(|_| invalid())
}

/// The house as an org id, or `None` for any other principal.
fn org_of(house: &PrincipalId) -> Option<u64> {
    match house {
        PrincipalId::Organization(org) => Some(org.0),
        _ => None,
    }
}

/// **The survey rows a site clerk is offered** (B8 item 31): one `"{site}/{l}"` row a held block
/// of the house.
fn survey_rows(w: &World, clerk: ObjId) -> Map<String, Value> {
    let mut rows = Map::new();
    let Some((map, site)) = crate::port::site::clerk_site(w, clerk) else {
        return rows;
    };
    let Some(yard) = site.yard.as_deref() else {
        return rows;
    };
    let Some(dispatcher) = yard_row(&map).and_then(|row| row.dispatcher) else {
        return rows;
    };
    let Some(house) = crate::port::site::org_principal(w, &yard.org)
        .as_ref()
        .and_then(org_of)
    else {
        return rows;
    };
    let Ok(context) = crate::surface::chart::site_context(w, &map.entry) else {
        return rows;
    };
    for letter in held_letters(&site, &yard.org) {
        let fields: Vec<u8> = crate::surface::plan::block_fields(letter)
            .into_iter()
            .filter_map(|(fx, fy)| crate::surface::address::field_index(fx, fy))
            .collect();
        rows.insert(
            format!("{}/{letter}", map.entry),
            json!({
                "site": map.entry.as_str(),
                "letter": letter.to_string(),
                "clerk": clerk.0,
                "dispatcher": dispatcher,
                "house": house,
                "body": context.spec.body.as_str(),
                "fields": fields,
            }),
        );
    }
    rows
}

/// **The delivery row a dispatcher is offered** (B8 item 31): its yard, hold, storefront and the
/// mill's day.
fn deliver_row(w: &World, dispatcher: ObjId) -> Option<Value> {
    let yard = yard_of_dispatcher(w, dispatcher)?;
    let hold = yard.row.hold?;
    let (storefront, mill) = house_mill(w, &yard)?;
    Some(json!({
        "site": yard.entry.as_str(),
        "dispatcher": dispatcher.0,
        "house": org_of(&yard.house)?,
        "good": yard.def.ore.as_str(),
        "hold": hold,
        "storefront": storefront.0,
        "bound_t": mill.per_ic_day_t,
    }))
}

impl ActionProvider for DispatchProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let w = &engine.world;
        let actor = query.actor();
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let parameter = |id: &str, kind: ProjectedParameterKind| {
            Ok::<_, mush_core::ActionError>(ProjectedParameter {
                id: CanonicalId::new(id)
                    .map_err(|_| integrity("the dispatch's parameter id is invalid"))?,
                required: true,
                kind,
            })
        };
        let (rows, schema) = {
            let surveys = survey_rows(w, actor);
            if surveys.is_empty() {
                let Some(row) = deliver_row(w, actor) else {
                    return Ok(None);
                };
                let mut rows = Map::new();
                rows.insert("deliver".to_owned(), row);
                (
                    rows,
                    vec![
                        parameter(
                            "act",
                            ProjectedParameterKind::Choice {
                                choices: vec!["deliver".to_owned()],
                            },
                        )?,
                        parameter(
                            "ordinal",
                            ProjectedParameterKind::Integer {
                                minimum: 1,
                                maximum: i64::from(u32::MAX),
                            },
                        )?,
                    ],
                )
            } else {
                let choices: Vec<String> = surveys.keys().cloned().collect();
                (
                    surveys,
                    vec![
                        parameter(
                            "act",
                            ProjectedParameterKind::Choice {
                                choices: vec!["survey".to_owned()],
                            },
                        )?,
                        parameter("row", ProjectedParameterKind::Choice { choices })?,
                    ],
                )
            }
        };
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the dispatch's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the dispatch's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the dispatch's definition guard is unavailable"))?;
        Ok(Some(ProviderOfferDraft {
            // The prepare has no world, so the IC instant rides the server-authenticated offer
            // beside the rows (the mill's precedent).
            fixed_parameters: json!({
                "rows": Value::Object(rows),
                "now": w.world_ic_now().at,
            }),
            parameters: ProjectedParameterSchema::new(schema)
                .map_err(|_| integrity("the dispatch's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-dispatch",
                "Dispatch a yard",
                "A site clerk surveys a house's held block, or a yard's dispatcher delivers its \
                 hold's ore to the house's storefront.",
            )
            .map_err(|_| integrity("the dispatch's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the dispatch's offer guards are invalid"))?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        let parameters = parameters(invocation)?;
        let now = parameters
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(invalid)?;
        let rows = parameters
            .get("rows")
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let (outcome, act) = match parameters.get("act").and_then(Value::as_str) {
            Some("survey") => (
                self.survey(invocation, txn, parameters, rows, now)?,
                "survey",
            ),
            Some("deliver") => (
                self.deliver(invocation, txn, parameters, rows, now)?,
                "deliver",
            ),
            _ => return Err(invalid()),
        };
        let (receipt, site, t) = outcome;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": act,
                "actor": invocation.actor().0,
                "receipt": receipt,
                "site": site,
                "t": t,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the dispatch's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl DispatchProvider {
    /// **`survey`'s prepare** (B8 item 31), nothing staged before its one write: the fixed row;
    /// the acting clerk its own; the block's record holding a claim of the house live at `now`;
    /// the dispatcher's chart row (absent → a fresh one); not all four fields already read; the
    /// seed written and checked; the row inserted or replaced. Answers `(receipt, site, 0)`.
    fn survey(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        parameters: &Map<String, Value>,
        rows: &Map<String, Value>,
        now: i64,
    ) -> Result<(String, String, u64), mush_core::ActionError> {
        let key = text(parameters, "row")?;
        let row = rows
            .get(key)
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let site = canonical(row, "site")?;
        let letter = text(row, "letter")?.chars().next().ok_or_else(invalid)?;
        let clerk = object_of(number(row, "clerk")?)?;
        let dispatcher = object_of(number(row, "dispatcher")?)?;
        let house = PrincipalId::Organization(OrgId(number(row, "house")?));
        let body = canonical(row, "body")?;
        let fields: Vec<u8> = row
            .get("fields")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?
            .iter()
            .map(|field| {
                field
                    .as_u64()
                    .and_then(|field| u8::try_from(field).ok())
                    .ok_or_else(invalid)
            })
            .collect::<Result<_, _>>()?;
        if clerk != invocation.actor() {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that survey is another clerk's",
            ));
        }
        let record = txn
            .get(
                &self.claims,
                &BlockKey {
                    site: site.clone(),
                    letter,
                },
            )
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the block records are unavailable",
                )
            })?;
        let held = record
            .and_then(|record| record.claim.as_ref())
            .is_some_and(|claim| claim.live(now) && claim.holder == house);
        if !held {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "that block is not the house's",
            ));
        }
        let chart_key = ChartKey {
            principal: dispatcher,
            site: site.clone(),
        };
        let prior = txn
            .get(&self.charts, &chart_key)
            .map_err(|_| port_error(ActionErrorCode::Integrity, "the charts are unavailable"))?
            .cloned();
        let existed = prior.is_some();
        let mut chart = prior.unwrap_or_else(|| ChartRecord::new(dispatcher, site.clone(), body));
        let surveyed = fields.iter().all(|index| {
            crate::surface::address::field_of_index(*index).is_some_and(|field| {
                crate::surface::chart::row_reads_field(Some(&chart), field).is_some()
            })
        });
        if surveyed {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that block is already surveyed",
            ));
        }
        crate::surface::chart::write_seed(&mut chart, &fields, now);
        chart.check().map_err(|_| {
            port_error(
                ActionErrorCode::ProviderRejected,
                "the survey is out of bounds",
            )
        })?;
        let staged = if existed {
            txn.replace(&self.charts, &chart_key, chart)
        } else {
            txn.insert(&self.charts, &chart_key, chart)
        };
        staged.map_err(|_| port_error(ActionErrorCode::Integrity, "the survey did not stage"))?;
        Ok((key.to_owned(), site.to_string(), 0))
    }

    /// **`deliver`'s prepare** (B8 item 31), nothing staged before step 7: (1) the fixed row, the
    /// acting dispatcher its own; (2) the ledger, its key re-checked; (3) only the ledger's next
    /// ordinal (the replay guard), and a present ledger's terms the row's; (4) the hold's and the
    /// storefront's ore (available, less holds); (5) [`deliverable`] — nothing due refuses; (6)
    /// the next ledger, checked; (7) one `transfer`, then the ledger inserted or replaced.
    /// Answers `(ordinal, site, t)`.
    #[allow(clippy::too_many_lines)] // one ordered prepare
    fn deliver(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        parameters: &Map<String, Value>,
        rows: &Map<String, Value>,
        now: i64,
    ) -> Result<(String, String, u64), mush_core::ActionError> {
        // (1) The row is the acting dispatcher's.
        let ordinal = number(parameters, "ordinal")?;
        let row = rows
            .get("deliver")
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let site = canonical(row, "site")?;
        let dispatcher = object_of(number(row, "dispatcher")?)?;
        let house = number(row, "house")?;
        let good = canonical(row, "good")?;
        let hold = VaultId(number(row, "hold")?);
        let storefront = VaultId(number(row, "storefront")?);
        let bound_t = number(row, "bound_t")?;
        if dispatcher != invocation.actor() {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that delivery is another dispatcher's",
            ));
        }
        // (2) The ledger, its key re-checked.
        let key = DispatchKey { site: site.clone() };
        let prior = txn
            .get(&self.dispatch, &key)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the dispatch ledgers are unavailable",
                )
            })?
            .cloned();
        if let Some(ledger) = &prior
            && ledger.site != site
        {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "a dispatch ledger is stored under another site's key",
            ));
        }
        // (3) The replay guard, then the terms.
        let next = prior
            .as_ref()
            .map_or(Some(1), |l| l.deliveries.checked_add(1));
        if next != Some(ordinal) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that delivery is already made",
            ));
        }
        if let Some(ledger) = &prior
            && (ledger.good != good || ledger.hold != hold || ledger.storefront != storefront)
        {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the yard's delivery terms no longer match its ledger",
            ));
        }
        // (4) The ore at either end.
        let unreadable = |_| {
            port_error(
                ActionErrorCode::ProviderRejected,
                "the yard's ore cannot be read",
            )
        };
        let hold_t = self
            .inventory
            .available(txn, EndpointRef(hold), &good)
            .map_err(unreadable)?;
        let front_t = self
            .inventory
            .available(txn, EndpointRef(storefront), &good)
            .map_err(unreadable)?;
        // (5) What is due.
        let t = deliverable(hold_t, front_t, bound_t);
        if t == 0 {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "nothing is due for delivery",
            ));
        }
        // (6) The next ledger.
        let overflow = || {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the delivery ledger's totals overflow",
            )
        };
        let today = crate::port::mill::ic_day(now);
        let (delivered_t, day_t, mut recent) = match &prior {
            Some(ledger) => (
                ledger.delivered_t.checked_add(t).ok_or_else(overflow)?,
                if ledger.day == today {
                    ledger.day_t.checked_add(t).ok_or_else(overflow)?
                } else {
                    t
                },
                ledger.recent.clone(),
            ),
            None => (t, t, Vec::new()),
        };
        recent.push(Delivery {
            ordinal,
            at_ic: now,
            t,
        });
        let excess = recent.len().saturating_sub(MAX_RECENT_DELIVERIES);
        recent.drain(..excess);
        let ledger = DispatchLedger {
            site: site.clone(),
            house,
            dispatcher,
            good: good.clone(),
            hold,
            storefront,
            deliveries: ordinal,
            delivered_t,
            day: today,
            day_t,
            recent,
        };
        ledger.check().map_err(|_| {
            port_error(
                ActionErrorCode::ProviderRejected,
                "the delivery ledger is out of bounds",
            )
        })?;
        // (7) The ore moved, in kind, then the ledger.
        self.inventory
            .transfer(txn, EndpointRef(hold), EndpointRef(storefront), &good, t)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::ProviderRejected,
                    "the yard's ore could not be delivered",
                )
            })?;
        let staged = if prior.is_some() {
            txn.replace(&self.dispatch, &key, ledger)
        } else {
            txn.insert(&self.dispatch, &key, ledger)
        };
        staged.map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the delivery ledger did not stage",
            )
        })?;
        Ok((ordinal.to_string(), site.to_string(), t))
    }
}

// ---- the clerk's seeds ---------------------------------------------------------------------

/// **What the house is still owed by the site clerk** (doc 20 cut SM.12b; B9 item 32), in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owed {
    /// A held block's record lacks a claim of the house live now (SM.10's item 7a's work).
    Seeds,
    /// The house's standing licence is not recorded (item 8).
    Licence,
    /// This held block's four fields are not all in the dispatcher's chart row (item 9).
    Survey(char),
}

/// **The one owed helper** (doc 20 cut SM.12b; 12b-I1) — the clerk ([`clerk_item`]), the
/// dispatcher's gate ([`dispatch_step`]) and the Dispatch line's Pending read it: `Seeds` while
/// any held letter's block record (`SurfaceReads::block_tenure`) lacks a claim live at `now` held
/// by the house; else `Licence` while the yard row records none; else `Survey(l)` for the first
/// held letter (all of them, letter order — "the house surveyed what it staked") whose four
/// fields the dispatcher's chart row does not all read; else `None`.
///
/// # Errors
///
/// `Err(())` on an unreadable read, or a yard with no dispatcher.
pub(crate) fn owed(
    reads: &dyn SurfaceReads,
    yard: &YardView<'_>,
    now: i64,
) -> Result<Option<Owed>, ()> {
    let letters = held_letters(yard.site, &yard.def.org);
    for letter in &letters {
        let record = reads
            .block_tenure(&BlockKey {
                site: yard.entry.clone(),
                letter: *letter,
            })
            .map_err(|_| ())?;
        let held = record
            .as_ref()
            .and_then(|record| record.claim.as_ref())
            .is_some_and(|claim| claim.live(now) && claim.holder == yard.house);
        if !held {
            return Ok(Some(Owed::Seeds));
        }
    }
    if yard.row.licence.is_none() {
        return Ok(Some(Owed::Licence));
    }
    let dispatcher = yard.dispatcher().ok_or(())?;
    let row = reads.chart(dispatcher, &yard.entry).map_err(|_| ())?;
    for letter in letters {
        let read = crate::surface::plan::block_fields(letter)
            .into_iter()
            .all(|field| crate::surface::chart::row_reads_field(row.as_ref(), field).is_some());
        if !read {
            return Ok(Some(Owed::Survey(letter)));
        }
    }
    Ok(None)
}

/// What the clerk's item needs of the yard, owned (so the step may hold `&mut Ctx`).
struct ClerkYard {
    entry: CanonicalId,
    house: PrincipalId,
    org: CanonicalId,
    owed: Result<Option<Owed>, ()>,
}

/// **The site clerk's items 8–9** (doc 20 cut SM.12b; B9 item 33; D16, D26), between SM.10's
/// held seeds (7a) and its paper seeds (7b): **8**, the house's standing licence (normal lane:
/// an existing house account adopted, else one opened `Active`, recorded on the yard row in the
/// same handler); **9**, one house survey a fire (`tacsim_dispatch` `survey` as the clerk, under
/// SM.6's attempt rule). **The papers never wait on the house**: this answers `None` — so 7b
/// runs on the same fire — whenever it cannot make progress: a site with no yard; nothing owed,
/// or the held seeds owed (7a's); an unreadable read (paged once); the dispatch action
/// unregistered (paged once); item 8's refusal (paged once; it retries every later fire); item
/// 9 after `MAX_SITE_ATTEMPTS` unlanded submissions while a paper is due (paged once; the survey
/// retries on every fire no paper is due). `Some(step)` only when it submitted, wrote, or its
/// swap was deferred (the clerk's one slot busy, so 7b could not submit either).
#[allow(clippy::too_many_lines)] // the two items, each with its refusals
pub(crate) fn clerk_item(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    clerk: ObjId,
    map: &PortMap,
    site: &SiteDef,
    now: i64,
    papers_due: bool,
) -> Option<Step> {
    site.yard.as_ref()?;
    let yard = {
        let reads: &dyn SurfaceReads = &*ctx;
        let found = yards(ctx.w)
            .into_iter()
            .find(|yard| yard.entry == map.entry)?;
        ClerkYard {
            entry: found.entry.clone(),
            house: found.house.clone(),
            org: found.def.org.clone(),
            owed: owed(reads, &found, now),
        }
    };
    let entry = yard.entry.clone();
    let owed = match yard.owed {
        Ok(None | Some(Owed::Seeds)) => return None,
        Ok(Some(owed)) => owed,
        Err(()) => {
            if crate::port::site::page_once(ctx.w, format!("yard/{entry}/owed")) {
                tracing::error!(
                    site = %entry,
                    runbook = "doc 20 cut SM.12b: the house's seeds cannot be read (a block \
                               record, the dispatcher's chart row, or no dispatcher); the papers \
                               go on and the house's licence and survey wait",
                    "tacsim: a yard's owed items cannot be read"
                );
            }
            return None;
        }
    };
    match owed {
        Owed::Seeds => None,
        Owed::Licence => clerk_licence(ctx, &yard, now),
        Owed::Survey(letter) => clerk_survey(ctx, fire, clerk, &entry, letter, papers_due),
    }
}

/// Item 8: the house licence (normal lane; 3 writes when opened, 1 when adopted).
fn clerk_licence(ctx: &mut Ctx<'_>, yard: &ClerkYard, now: i64) -> Option<Step> {
    let entry = &yard.entry;
    let refused = |ctx: &mut Ctx<'_>, why: &str| {
        if crate::port::site::page_once(ctx.w, format!("yard/{entry}/licence")) {
            tracing::error!(
                site = %entry,
                why,
                runbook = "doc 20 cut SM.12b: the house's standing licence cannot be recorded; \
                           the papers go on and the clerk retries every fire",
                "tacsim: a yard's house licence was not recorded"
            );
        }
        None
    };
    let Some(until_ic) = now.checked_add(crate::surface::tenure::SEEDED_TERM_IC_SECS) else {
        return refused(ctx, "the term overflows");
    };
    let Some(org_row) = crate::port::site::map_org(ctx.w, &yard.org) else {
        return refused(ctx, "the house has no lane org row");
    };
    let Ok(lane) = CanonicalId::new(crate::definitions::content::LANE_PACK) else {
        return refused(ctx, "the lane id");
    };
    let Some(mut next) =
        ctx.w
            .module_state::<TacsimStore>()
            .and_then(|store| match store.port_map(&lane, entry) {
                PortMapState::Present(map) => Some(map.clone()),
                PortMapState::Absent | PortMapState::Unreadable => None,
            })
    else {
        return refused(ctx, "the site's port map cannot be read");
    };
    let account = match crate::port::licence::house_account(ctx.w, &yard.house) {
        Some((account, _)) => account,
        None => {
            let name = HOUSE_LICENCE_NAME.replace("{org}", &org_name(ctx.w, &yard.org));
            let Some(account) = crate::port::licence::open_house_licence(
                ctx,
                &yard.house,
                OrgScopeId(org_row.root_scope),
                &name,
            ) else {
                return refused(ctx, "the account allocator refused");
            };
            if crate::port::licence::house_account(ctx.w, &yard.house).map(|(id, _)| id)
                != Some(account)
            {
                return refused(ctx, "the account did not open");
            }
            account
        }
    };
    yard_row_mut(&mut next).licence = Some(crate::port::map::HouseLicence {
        account: account.0,
        from_ic: now,
        until_ic,
    });
    put_port_map(ctx, &next);
    Some(Step::Worked)
}

/// Item 9: one house survey, under SM.6's attempt rule.
fn clerk_survey(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    clerk: ObjId,
    entry: &CanonicalId,
    letter: char,
    papers_due: bool,
) -> Option<Step> {
    let lower = letter.to_ascii_lowercase();
    let attempt_key = format!("survey/{entry}/{letter}");
    let attempts = crate::port::site::attempt(ctx.w, &attempt_key);
    if attempts >= crate::port::site::MAX_SITE_ATTEMPTS && !ctx.authoritative_pending(clerk) {
        if crate::port::site::page_once(ctx.w, attempt_key.clone()) {
            tracing::error!(
                site = %entry,
                %letter,
                attempts,
                runbook = "doc 20 cut SM.12b: the house's survey of a held block never lands; the \
                           papers go on and the survey retries whenever no paper is due",
                "tacsim: a yard's house survey never lands"
            );
        }
        if papers_due {
            return None;
        }
    }
    let Some(action) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|handles| handles.dispatch.clone())
    else {
        if crate::port::site::page_once(ctx.w, format!("yard/{entry}/dispatch")) {
            tracing::error!(
                site = %entry,
                "tacsim: the dispatch action is not registered (no inventory cell); the house's \
                 survey waits and the papers go on"
            );
        }
        return None;
    };
    let Ok(key) =
        IdempotencyKeyText::parse_canonical(format!("{DISPATCH_KEY_PREFIX}{entry}/survey/{lower}"))
    else {
        return None;
    };
    let row = format!("{entry}/{letter}");
    let step = crate::port::stock::swapped(fire.as_principal(ctx, clerk, clerk, |ctx| {
        ctx.submit_authoritative(
            action,
            PerceivedRef::SelfActor,
            Vec::new(),
            json!({ "act": "survey", "row": row }),
            key,
        );
    }));
    if step == Step::Submitted {
        crate::port::site::count_attempt(ctx.w, &attempt_key);
    }
    Some(step)
}

// ---- the dispatcher ------------------------------------------------------------------------

/// **When a yard rig's next plan is due** (doc 20 cut SM.12b; O-14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileDue {
    /// Now.
    Now,
    /// Once the IC clock reaches this instant.
    At(i64),
    /// Never by itself (a running plan, or an end the house does not re-arm after).
    Never,
}

/// **The re-arm rule** (doc 20 cut SM.12b; O-14, ruled by the owner 2026-10-24: the default kept
/// provisionally), pure: no plan on the record (never filed, or struck by staff) → `Now`; a
/// `Live` or `Parking` plan → `Never` (it runs); `Ended { HoldFull | Bound }` → `Now` once
/// `at_ic + rearm ≤ now` (saturating), else `At(at_ic + rearm)`; `Ended { WorkedOut }` → as
/// `HoldFull` **only when `fair_left` and the ended plan landed a cycle**, else `Never` — **the
/// house re-arms only on ore it has read** ([`fair_left`] reads the house's own chart row);
/// `Ended` on `Cleared`, a right (`Licence`, `Claimed`, `Lease`, `Dark`, `ChartFull`) or
/// `Stranded` → `Never`. SM.11's plan record never re-arms by itself for a player.
#[must_use]
pub(crate) fn file_due(
    plan: Option<&FillPlan>,
    rearm_ic_secs: i64,
    now: i64,
    fair_left: bool,
) -> FileDue {
    let Some(plan) = plan else {
        return FileDue::Now;
    };
    let after = |at_ic: i64| {
        let due = at_ic.saturating_add(rearm_ic_secs);
        if due <= now {
            FileDue::Now
        } else {
            FileDue::At(due)
        }
    };
    match plan.state {
        FillState::Live | FillState::Parking { .. } => FileDue::Never,
        FillState::Ended { why, at_ic } => match why {
            PlanEnd::HoldFull | PlanEnd::Bound => after(at_ic),
            PlanEnd::WorkedOut if fair_left && plan.tally.cycles >= 1 => after(at_ic),
            PlanEnd::WorkedOut
            | PlanEnd::Cleared
            | PlanEnd::Licence { .. }
            | PlanEnd::Claimed
            | PlanEnd::Lease { .. }
            | PlanEnd::Dark
            | PlanEnd::ChartFull
            | PlanEnd::Stranded => FileDue::Never,
        },
    }
}

/// **Whether fair ore is left in `blocks`** (doc 20 cut SM.12b), pure: some pocket of `blocks`
/// (`address::pocket_of_block_index`) whose class in the family reads fair or better and is not
/// worked out — **exactly `decide`'s Block-step predicate** (`crate::surface::plan`) less its
/// passed pockets. `classes` is the house's own chart row's reading (a pocket it never sounded
/// reads `None`), so the house re-arms only on ore it has read.
#[must_use]
pub(crate) fn fair_left(
    classes: impl Fn((i64, i64)) -> Option<Word>,
    pit_out: impl Fn((i64, i64)) -> bool,
    blocks: &[char],
) -> bool {
    let pockets = usize::try_from(crate::surface::address::POCKETS_IN_BLOCK).unwrap_or(256);
    blocks.iter().any(|letter| {
        (0..pockets).any(|index| {
            crate::surface::address::pocket_of_block_index(*letter, index).is_some_and(|pocket| {
                classes(pocket).is_some_and(|class| class >= Word::Fair) && !pit_out(pocket)
            })
        })
    })
}

/// **Why the dispatcher warned** (doc 20 cut SM.12b): each kind its own key, once a boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DispatchWarn {
    /// A read the dispatcher needs cannot be read.
    Unreadable,
    /// An act was refused (a delivery that never lands; a compile's refusal).
    Refused,
    /// A rig is stalled: its last unload left ore in its hopper.
    Stalled,
}

/// Warn once a boot under `(dispatcher, kind, what)`, at most `MAX_CLERK_ROWS` keys.
fn dispatch_warn(state: &mut PortTickState, dispatcher: ObjId, kind: DispatchWarn, what: &str) {
    let key = (dispatcher, kind, SmolStr::new(what));
    if state.dispatch_warned.contains(&key)
        || state.dispatch_warned.len() >= crate::port::stock::MAX_CLERK_ROWS
    {
        return;
    }
    state.dispatch_warned.insert(key);
    match kind {
        DispatchWarn::Unreadable => tracing::warn!(
            dispatcher = dispatcher.0,
            what,
            runbook = "@tacsim/port status",
            "tacsim: a yard's dispatcher cannot read what it needs; it passes that act over"
        ),
        DispatchWarn::Refused => tracing::warn!(
            dispatcher = dispatcher.0,
            what,
            runbook = "@tacsim/port status",
            "tacsim: a yard's dispatcher act was refused; it backs that act off"
        ),
        DispatchWarn::Stalled => tracing::warn!(
            dispatcher = dispatcher.0,
            rig = what,
            runbook = "doc 20 cut SM.12b: unload the rig as the dispatcher (+tv/<n> unload go); \
                       the dispatcher resumes it by itself once its hopper is empty",
            "tacsim: a yard rig holds ore its last unload left in the hopper; no plan is filed on it"
        ),
    }
}

/// A refusal's variant name, for a warning's key (no world read, no figure).
fn variant_word<T: std::fmt::Debug>(value: &T) -> String {
    let debug = format!("{value:?}");
    debug
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// What the dispatcher's step needs of its yard, owned (so it may hold `&mut Ctx`).
struct StepYard {
    entry: CanonicalId,
    hull: ObjId,
    hold: Option<VaultId>,
    ore: CanonicalId,
    blocks: Vec<char>,
    rearm: i64,
    mill: Option<(VaultId, u64)>,
}

/// **The dispatcher's step** (doc 20 cut SM.12b; B10 item 40) — at most **one act a fire** on its
/// own `Arm::Dispatch`, reading only durable state and the IC clock, in this order:
///
/// - gone → `Refused`; no yard → `Idle`; **its one submission slot busy** → `Deferred` before any
///   read (the slot is shared with both rigs' plan steps and queued unloads on the surface tick,
///   so its throughput is below two players with one rig each, and the mill's 40 t an IC day
///   binds first — 12b-E12); the site not open → `Idle`; the clerk's items owed (or unreadable)
///   → `Idle` (the seeds, the licence and the survey come first).
/// - **(a) deliver first** (it makes room for the next plan): [`deliverable`] tonnes of the yard
///   hold's ore to the house storefront, under the ledger's next ordinal
///   (`tacsim.dispatch/{site}/deliver/{n}`), SM.12a's attempt rule (the same ordinal submitted
///   [`MAX_DISPATCH_ATTEMPTS`] times with no advance → passed over for the IC day, warned once).
/// - **(b) else file one rig's plan**: the first yard rig in slot order that is quiet (no act, no
///   run, no queued unload, no live plan) in the yard's bay, whose [`file_due`] is `Now` and whose
///   back-off has passed (the blocks' classes, [`fair_left`], are read only then and only for a
///   `WorkedOut` end with a cycle; a read with no fair ore backs the rig off one interval, so
///   worked-out blocks are never re-read every fire; a cold ground is wanted and the next rig
///   tried) — **and not stalled**: a rig whose last unload left ore in a hopper that
///   cannot take a cycle (`crate::surface::plan::bay_hopper` reads `HoldFull`, its hopper not
///   empty) is never re-filed — a new plan would end `HoldFull` at once without launching, and
///   `record.last` changes only when an act lands on the rig, so re-testing the hold's room can
///   never unstick it; only an unload can. It is warned once, skipped, and named by the Dispatch
///   line (order 5d) with its staff remedy, `+tv/{n} unload go` typed as the dispatcher (an
///   ordinary SM.8 hand unload; no new verb); since this check reads `record.last` afresh every
///   fire, the rig resumes by itself on the first fire after that unload lands with nothing left
///   in the hopper. The compile is `crate::surface::act::plan_ask` over `yard.blocks` and its
///   ore's family, the filing `crate::surface::act::file_compiled` as the dispatcher. A cold
///   ground is wanted (`Deferred`); **any other refusal backs that rig off one re-arm interval**
///   (D21: the compile never re-runs every fire), warned once a boot.
///
/// The plan's start, steps, parking, end, notes and tally are SM.11's, unchanged; the dispatcher
/// never writes a plan itself, and its lines go nowhere (`queue_notice` is players only; D12).
#[allow(clippy::too_many_lines)] // one ordered decision
pub(crate) fn dispatch_step(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    fire: &mut Fire,
    dispatcher: ObjId,
) -> Step {
    use crate::surface::act::{Refusal, RigAt, Where};
    if crate::port_tick::gone(ctx.w, dispatcher) {
        return Step::Refused;
    }
    let now = ctx.w.world_ic_now().at;
    // The slot's state is read first, but answered after the yard is found (no yard → `Idle`):
    // the house mill, the clerk's items and the opening are read only once the slot is free (the
    // fixer round's conformance/F4 — the slot is busy on most fires while the drones work).
    let pending = ctx.authoritative_pending(dispatcher);
    let yard = {
        let reads: &dyn SurfaceReads = &*ctx;
        let Some(found) = yard_of_dispatcher(ctx.w, dispatcher) else {
            return Step::Idle;
        };
        let Some(hull) = found.hull() else {
            return Step::Idle;
        };
        if pending {
            return Step::Deferred;
        }
        if !matches!(
            crate::surface::opening::opening_line(ctx.w, &found.entry),
            crate::surface::opening::SiteOpening::Open { .. }
        ) {
            return Step::Idle;
        }
        // The clerk's items come first (or an unreadable read: wait for it).
        if !matches!(owed(reads, &found, now), Ok(None)) {
            return Step::Idle;
        }
        StepYard {
            entry: found.entry.clone(),
            hull,
            hold: found.row.hold.map(VaultId),
            ore: found.def.ore.clone(),
            blocks: found.def.blocks.clone(),
            rearm: found.def.rearm_ic_secs,
            mill: house_mill(ctx.w, &found).map(|(front, mill)| (front, mill.per_ic_day_t)),
        }
    };
    let today = crate::port::mill::ic_day(now);
    // (a) Deliver first.
    'deliver: {
        let Some((storefront, bound_t)) = yard.mill else {
            dispatch_warn(
                state,
                dispatcher,
                DispatchWarn::Unreadable,
                "the house mill",
            );
            break 'deliver;
        };
        let Some(hold) = yard.hold else {
            dispatch_warn(state, dispatcher, DispatchWarn::Unreadable, "the yard hold");
            break 'deliver;
        };
        if state.dispatch_refused.get(&dispatcher) == Some(&today) {
            break 'deliver;
        }
        let (Ok(hold_t), Ok(front_t)) = (
            crate::port_tick::read_available(ctx, EndpointRef(hold), &yard.ore),
            crate::port_tick::read_available(ctx, EndpointRef(storefront), &yard.ore),
        ) else {
            dispatch_warn(state, dispatcher, DispatchWarn::Unreadable, "the ore pools");
            break 'deliver;
        };
        if deliverable(hold_t, front_t, bound_t) == 0 {
            break 'deliver;
        }
        let Some(handle) = ctx
            .w
            .module_state::<crate::port::families::PortHandles>()
            .map(|handles| handles.families.dispatch.clone())
        else {
            break 'deliver;
        };
        let key = DispatchKey {
            site: yard.entry.clone(),
        };
        let ledger = match dispatch_at(ctx, &handle, &key) {
            Ok(ledger) => ledger,
            Err(()) => {
                dispatch_warn(state, dispatcher, DispatchWarn::Unreadable, "the ledger");
                refuse_today(state, dispatcher, today);
                break 'deliver;
            }
        };
        let Some(ordinal) = ledger
            .as_ref()
            .map_or(Some(1), |ledger| ledger.deliveries.checked_add(1))
        else {
            break 'deliver;
        };
        let attempts = match state.dispatch_attempts.get(&dispatcher) {
            Some((recorded, attempts)) if *recorded == ordinal => *attempts,
            Some(_) => {
                state.dispatch_attempts.remove(&dispatcher);
                0
            }
            None => 0,
        };
        if attempts >= MAX_DISPATCH_ATTEMPTS {
            state.dispatch_attempts.remove(&dispatcher);
            refuse_today(state, dispatcher, today);
            dispatch_warn(state, dispatcher, DispatchWarn::Refused, "deliver");
            break 'deliver;
        }
        let Some(action) = state.dispatch_action.clone() else {
            break 'deliver;
        };
        let Ok(idempotency) = IdempotencyKeyText::parse_canonical(format!(
            "{DISPATCH_KEY_PREFIX}{}/deliver/{ordinal}",
            yard.entry
        )) else {
            return Step::Refused;
        };
        let step =
            crate::port::stock::swapped(fire.as_principal(ctx, dispatcher, dispatcher, |ctx| {
                ctx.submit_authoritative(
                    action,
                    PerceivedRef::SelfActor,
                    Vec::new(),
                    json!({ "act": "deliver", "ordinal": ordinal }),
                    idempotency,
                );
            }));
        if step == Step::Submitted
            && (state.dispatch_attempts.contains_key(&dispatcher)
                || state.dispatch_attempts.len() < crate::port::stock::MAX_CLERK_ROWS)
        {
            state
                .dispatch_attempts
                .insert(dispatcher, (ordinal, attempts.saturating_add(1)));
        }
        return step;
    }
    // (b) File one rig's plan.
    let Some(family) = crate::surface::geology::family_of_word(yard.ore.as_str()) else {
        return Step::Idle;
    };
    let Some(tools) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|handles| handles.surface_tools.clone())
    else {
        return Step::Idle;
    };
    let list: String = yard
        .blocks
        .iter()
        .map(char::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let pool_unit = crate::port::tv::pool_unit(ctx.w);
    // A rig passed over on a cold ground: the fire answers `Deferred` if no other rig files.
    let mut cold = false;
    for lease in house_rigs(ctx.w, dispatcher) {
        let tv = lease.tv;
        let record = match SurfaceReads::rig(&*ctx, tv) {
            Ok(record) => {
                record.unwrap_or_else(|| crate::surface::excursion::RigRecord::fresh_unread(tv))
            }
            Err(_) => {
                dispatch_warn(state, dispatcher, DispatchWarn::Unreadable, &lease.mark);
                continue;
            }
        };
        if record.act.is_some()
            || record.run.is_some()
            || record.queued.is_some()
            || record.live_plan_for(dispatcher).is_some()
        {
            continue;
        }
        if crate::surface::act::where_is(ctx.w, tv) != (Where::Bay { hull: yard.hull }) {
            continue;
        }
        // Never re-arm into a known dead end (12b-T2; the reviewer's F4).
        if crate::surface::plan::bay_hopper(record.last.as_ref())
            == crate::surface::plan::BayHopper::HoldFull
        {
            let read = |endpoint: EndpointRef, good: &CanonicalId| {
                crate::port_tick::read_pool(ctx, endpoint, good)
            };
            match crate::port::tv::hopper_load_with(ctx.w, tv, pool_unit.as_ref(), &read) {
                Ok((0, _)) => {}
                Ok(_) => {
                    dispatch_warn(state, dispatcher, DispatchWarn::Stalled, &lease.mark);
                    continue;
                }
                Err(_) => {
                    dispatch_warn(state, dispatcher, DispatchWarn::Unreadable, &lease.mark);
                    continue;
                }
            }
        }
        // The rule first, as if ore were left (the cheap reads: the record and the clock), then
        // the back-off, and only then — for a `WorkedOut` end with a cycle, the one end whose
        // re-arm depends on the ground — the blocks' classes (the fixer round's correctness/F6:
        // `fair_left` reads both blocks' 512 pockets and the pits, so it is never read inside the
        // interval, inside a back-off, or every fire once the blocks read worked out).
        if file_due(record.plan.as_ref(), yard.rearm, now, true) != FileDue::Now {
            continue;
        }
        if state
            .dispatch_file_after
            .get(&tv)
            .is_some_and(|after| *after > now)
        {
            continue;
        }
        let worked_out = record.plan.as_ref().is_some_and(|plan| {
            matches!(
                plan.state,
                FillState::Ended {
                    why: PlanEnd::WorkedOut,
                    ..
                }
            ) && plan.tally.cycles >= 1
        });
        if worked_out {
            let fair = match fair_left_now(ctx, dispatcher, &yard.entry, family, &yard.blocks) {
                FairRead::Read(fair) => fair,
                FairRead::Cold => {
                    // Wanted; the next rig may still be filed this fire.
                    crate::surface::want_site_ground(ctx.w, &yard.entry);
                    cold = true;
                    continue;
                }
                FairRead::Dark => {
                    dispatch_warn(state, dispatcher, DispatchWarn::Unreadable, "the ground");
                    false
                }
            };
            if file_due(record.plan.as_ref(), yard.rearm, now, fair) != FileDue::Now {
                // No fair ore read: the verdict stands until a new plan reads more, so it is
                // not read again for one re-arm interval (D21's back-off, unwarned).
                back_off_rig(state, tv, now, yard.rearm);
                continue;
            }
        }
        let rig = RigAt {
            tv,
            slot: lease.slot,
            mark: lease.mark.clone(),
            hull: lease.hull,
            record: lease.clone(),
        };
        let mut peeks = crate::surface::Peeks::default();
        crate::surface::plan::count_house_compile(ctx.w);
        let asked = crate::surface::act::plan_ask(
            ctx.w,
            &*ctx,
            dispatcher,
            &rig,
            &list,
            Some(family),
            &tools,
            &mut peeks,
            now,
        );
        let back_off = |state: &mut PortTickState, word: &str| {
            back_off_rig(state, tv, now, yard.rearm);
            dispatch_warn(
                state,
                dispatcher,
                DispatchWarn::Refused,
                &format!("{} {word}", lease.mark),
            );
        };
        return match asked {
            Err(refusal @ Refusal::GroundCold { .. }) => {
                crate::surface::act::want_cold(ctx, &rig, &refusal);
                Step::Deferred
            }
            Err(refusal) => {
                back_off(state, &variant_word(&refusal));
                Step::Idle
            }
            Ok((Err(refusal), _)) => {
                back_off(state, &variant_word(&refusal));
                Step::Idle
            }
            Ok((Ok(compiled), facts)) => {
                let filed = fire.as_principal(ctx, dispatcher, dispatcher, |ctx| {
                    crate::surface::act::file_compiled(
                        ctx,
                        &rig,
                        &list,
                        Some(family),
                        compiled,
                        &facts,
                    )
                });
                match filed {
                    Ok(Ok(_)) => Step::Submitted,
                    Ok(Err(_)) => {
                        dispatch_warn(state, dispatcher, DispatchWarn::Refused, "file");
                        Step::Idle
                    }
                    Err(refused) => crate::port::stock::swapped(Err(refused)),
                }
            }
        };
    }
    if cold { Step::Deferred } else { Step::Idle }
}

/// Back `tv`'s filing off one re-arm interval (D21; expired back-offs pruned first, at most
/// `MAX_CLERK_ROWS` rigs).
fn back_off_rig(state: &mut PortTickState, tv: ObjId, now: i64, rearm: i64) {
    state.dispatch_file_after.retain(|_, after| *after > now);
    if state.dispatch_file_after.contains_key(&tv)
        || state.dispatch_file_after.len() < crate::port::stock::MAX_CLERK_ROWS
    {
        state
            .dispatch_file_after
            .insert(tv, now.saturating_add(rearm));
    }
}

/// Pass the dispatcher's delivery over for the rest of `today` (stale days pruned first).
fn refuse_today(state: &mut PortTickState, dispatcher: ObjId, today: i64) {
    state.dispatch_refused.retain(|_, day| *day == today);
    if state.dispatch_refused.contains_key(&dispatcher)
        || state.dispatch_refused.len() < crate::port::stock::MAX_CLERK_ROWS
    {
        state.dispatch_refused.insert(dispatcher, today);
    }
}

/// What [`fair_left_now`] could read.
enum FairRead {
    /// The answer.
    Read(bool),
    /// The site's ground is cold: want it.
    Cold,
    /// The site's ground or the house's chart row cannot be read.
    Dark,
}

/// **[`fair_left`] over the world** (doc 20 cut SM.12b), the closures built as `decide` builds
/// them: the site context, the ground through the non-building peek, the dispatcher's own chart
/// row's classes in the family (`plan::read_classes` → `geology::by_family`) and the public pits
/// (`site_map::pit_exhausted`).
fn fair_left_now(
    ctx: &Ctx<'_>,
    dispatcher: ObjId,
    entry: &CanonicalId,
    family: OreFamily,
    blocks: &[char],
) -> FairRead {
    let reads: &dyn SurfaceReads = ctx;
    let mut peeks = crate::surface::Peeks::default();
    let ground = match crate::surface::chart::peek_site(ctx.w, entry, &mut peeks) {
        crate::surface::chart::SitePeek::Warm(ground) => ground,
        crate::surface::chart::SitePeek::Cold => return FairRead::Cold,
        crate::surface::chart::SitePeek::Dark => return FairRead::Dark,
    };
    let Ok(site) = crate::surface::chart::site_context(ctx.w, entry) else {
        return FairRead::Dark;
    };
    let Ok(row) = reads.chart(dispatcher, entry) else {
        return FairRead::Dark;
    };
    let cache = std::cell::RefCell::new(BTreeMap::new());
    let classes = |pocket: (i64, i64)| {
        crate::surface::plan::read_classes(&site, row.as_ref(), pocket, &mut cache.borrow_mut())
            .map(|classes| crate::surface::geology::by_family(classes, family))
    };
    let pit_out = |pocket: (i64, i64)| {
        crate::port::site_map::pit_exhausted(reads, entry, &ground, pocket, family)
    };
    FairRead::Read(fair_left(classes, pit_out, blocks))
}

// ---- readiness: the Dispatch line ----------------------------------------------------------

/// **What the readiness report read of one yard** (doc 20 cut SM.12b; the `MillLineRead` shape,
/// so each line is tested without a world), in the line's order of decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DispatchRead {
    /// Order 1: not laid up, or a recorded object gone.
    Missing,
    /// Order 2: `what` cannot be read.
    Unreadable {
        /// `rig {n}'s record` | `rig {n}'s hopper` | `the delivery ledger` | `the yard hold` |
        /// `the storefront`.
        what: String,
    },
    /// Order 3: no licence instrument (an unreachable guard).
    NoInstrument,
    /// Order 4: the house's licence account is not active.
    LicenceInactive,
    /// Order 5a: a rig stranded on the ground.
    Stranded {
        /// Its slot.
        n: u8,
        /// Its mark.
        mark: String,
    },
    /// Order 5b: a rig whose plan stopped on a right, the dark registry or a full chart — or
    /// ended stranded, the rig since brought back into the bay by staff (5a's remedy followed).
    Stopped {
        /// Its slot.
        n: u8,
        /// Its mark.
        mark: String,
        /// Why.
        why: PlanEnd,
    },
    /// Order 5c: a rig whose plan staff cleared.
    Held {
        /// Its slot.
        n: u8,
        /// Its mark.
        mark: String,
    },
    /// Order 5d: a rig stalled with ore its last unload left in the hopper.
    Stalled {
        /// Its slot.
        n: u8,
        /// Its mark.
        mark: String,
    },
    /// Order 6: the site not open.
    NotOpen {
        /// The rigs in the bay.
        rigs: u64,
        /// The yard's name.
        hull: String,
    },
    /// Order 7: the clerk's items owed.
    Owed,
    /// Order 8: every rig's last plan worked out, none re-armed.
    WorkedOut {
        /// The rigs.
        rigs: u64,
        /// The yard's name.
        hull: String,
        /// The ore good.
        ore: CanonicalId,
        /// Tonnes delivered in all.
        total_t: u64,
    },
    /// Order 9: working.
    Working {
        /// Rigs with an open excursion.
        out: u64,
        /// The rigs.
        rigs: u64,
        /// Rigs with a live plan.
        working: u64,
        /// Plans filed in all (Σ `RigRecord::plans`).
        filed: u64,
        /// Tonnes delivered today.
        day_t: u64,
        /// The ore good.
        ore: CanonicalId,
        /// Tonnes delivered in all.
        total_t: u64,
    },
    /// Order 10: nothing yet.
    Nothing {
        /// The rigs.
        rigs: u64,
        /// The yard's name.
        hull: String,
    },
}

/// **The Dispatch line's words** (doc 20 cut SM.12b; O-18's texts, verbatim), pure: `entry` the
/// site entry, `org` the house's org id.
#[must_use]
pub(crate) fn dispatch_line_text(
    entry: &CanonicalId,
    org: &CanonicalId,
    read: &DispatchRead,
) -> (LineState, String) {
    use crate::port::tv::{count_words, ore_words};
    let head = |template: &str| {
        template
            .replace("{entry}", entry.as_str())
            .replace("{org}", org.as_str())
    };
    let rig = |template: &str, n: u8, mark: &str| {
        head(template)
            .replace("{n}", &n.to_string())
            .replace("{mark}", mark)
    };
    match read {
        DispatchRead::Missing => (LineState::Missing, head(DISPATCH_MISSING)),
        DispatchRead::Unreadable { what } => (
            LineState::Flagged,
            head(DISPATCH_UNREADABLE).replace("{what}", what),
        ),
        DispatchRead::NoInstrument => (LineState::Flagged, head(DISPATCH_NO_INSTRUMENT)),
        DispatchRead::LicenceInactive => (LineState::Flagged, head(DISPATCH_LICENCE_INACTIVE)),
        DispatchRead::Stranded { n, mark } => {
            (LineState::Flagged, rig(DISPATCH_STRANDED, *n, mark))
        }
        DispatchRead::Stopped { n, mark, why } => (
            LineState::Flagged,
            rig(DISPATCH_STOPPED, *n, mark).replace("{why}", why.word()),
        ),
        DispatchRead::Held { n, mark } => (LineState::Flagged, rig(DISPATCH_HELD, *n, mark)),
        DispatchRead::Stalled { n, mark } => (LineState::Flagged, rig(DISPATCH_STALLED, *n, mark)),
        DispatchRead::NotOpen { rigs, hull } => (
            LineState::Ready,
            head(DISPATCH_NOT_OPEN)
                .replace("{rigs}", &count_words(*rigs, "rig", "rigs"))
                .replace("{hull}", hull),
        ),
        DispatchRead::Owed => (LineState::Pending, head(DISPATCH_OWED)),
        DispatchRead::WorkedOut {
            rigs,
            hull,
            ore,
            total_t,
        } => (
            LineState::Ready,
            head(DISPATCH_WORKED_OUT)
                .replace("{ore}", &ore_words(ore.as_str()))
                .replace("{rigs}", &count_words(*rigs, "rig", "rigs"))
                .replace("{hull}", hull)
                .replace("{total_t}", &total_t.to_string()),
        ),
        DispatchRead::Working {
            out,
            rigs,
            working,
            filed,
            day_t,
            ore,
            total_t,
        } => (
            LineState::Ready,
            head(DISPATCH_WORKING)
                .replace("{out}", &out.to_string())
                .replace("{rigs}", &rigs.to_string())
                .replace("{working}", &working.to_string())
                .replace("{filed}", &count_words(*filed, "plan", "plans"))
                .replace("{day_t}", &day_t.to_string())
                .replace("{ore}", &ore_words(ore.as_str()))
                .replace("{total_t}", &total_t.to_string()),
        ),
        DispatchRead::Nothing { rigs, hull } => (
            LineState::Ready,
            head(DISPATCH_NOTHING)
                .replace("{rigs}", &count_words(*rigs, "rig", "rigs"))
                .replace("{hull}", hull),
        ),
    }
}

/// **One yard's Dispatch readiness line** (doc 20 cut SM.12b; B11 item 42): reported, never gated
/// (`Binding::Dispatch`). It names the yard's state and never fixes it (D17).
#[must_use]
pub(crate) fn dispatch_line(
    ctx: &Ctx<'_>,
    entry: &CanonicalId,
    site: &SiteDef,
    yard: &YardDef,
    map: &PortMap,
) -> (LineState, String) {
    let read = dispatch_line_read(ctx, entry, site, yard, map);
    dispatch_line_text(entry, &yard.org, &read)
}

/// One rig's reading for the line: its slot, mark and record.
struct LineRig {
    n: u8,
    mark: String,
    tv: ObjId,
    record: Option<crate::surface::excursion::RigRecord>,
}

#[allow(clippy::too_many_lines)] // the line's ten orders, decided in turn
fn dispatch_line_read(
    ctx: &Ctx<'_>,
    entry: &CanonicalId,
    site: &SiteDef,
    yard: &YardDef,
    map: &PortMap,
) -> DispatchRead {
    use crate::surface::opening::{SiteOpening, opening_line};
    let w: &World = ctx.w;
    let reads: &dyn SurfaceReads = ctx;
    let now = w.world_ic_now().at;
    // 1. Laid up, every recorded object live.
    let Some(row) = yard_row(map) else {
        return DispatchRead::Missing;
    };
    let house = crate::port::site::org_principal(w, &yard.org);
    let hold_ok = row.hold.is_some_and(|hold| {
        w.module_state::<EconomyStore>().is_some_and(|economy| {
            economy
                .snapshot()
                .vault(VaultId(hold))
                .is_some_and(|vault| {
                    Some(&vault.owner) == house.as_ref() && vault.status == VaultStatus::Active
                })
        })
    });
    let laid = row.hulls.len() == 1
        && row.hulls.iter().all(|hull| live(w, hull.hull))
        && row.rigs.len() == usize::try_from(yard.rigs).unwrap_or(usize::MAX)
        && row.rigs.iter().all(|rig| live(w, rig.tv))
        && row.dispatcher.is_some_and(|dispatcher| live(w, dispatcher))
        && hold_ok
        && house.is_some();
    let (Some(house), Some(dispatcher), Some(hull), Some(hold), true) = (
        house,
        row.dispatcher.map(ObjId),
        row.hulls.first(),
        row.hold.map(VaultId),
        laid,
    ) else {
        return DispatchRead::Missing;
    };
    let hull_name = crate::port::coop::name_of(w, ObjId(hull.hull));
    let rigs = u64::from(yard.rigs);
    // 2. The reads: each rig's record (and, for a rig whose last unload left ore, its hopper),
    // the ledger, the hold, the storefront.
    let mut line_rigs: Vec<LineRig> = Vec::new();
    for view in house_views(row, dispatcher) {
        let record = match reads.rig(view.tv) {
            Ok(record) => record,
            Err(_) => {
                return DispatchRead::Unreadable {
                    what: format!("rig {}'s record", view.slot),
                };
            }
        };
        line_rigs.push(LineRig {
            n: view.slot,
            mark: view.mark.clone(),
            tv: view.tv,
            record,
        });
    }
    let pool_unit = crate::port::tv::pool_unit(w);
    let mut stalled: Vec<bool> = Vec::with_capacity(line_rigs.len());
    for rig in &line_rigs {
        let in_bay = crate::surface::act::where_is(w, rig.tv)
            == (crate::surface::act::Where::Bay {
                hull: ObjId(hull.hull),
            });
        let quiet = rig.record.as_ref().is_none_or(|record| {
            record.act.is_none()
                && record.run.is_none()
                && record.queued.is_none()
                && record.live_plan_for(dispatcher).is_none()
        });
        let hold_full = rig.record.as_ref().is_some_and(|record| {
            crate::surface::plan::bay_hopper(record.last.as_ref())
                == crate::surface::plan::BayHopper::HoldFull
        });
        if in_bay && quiet && hold_full {
            let read = |endpoint: EndpointRef, good: &CanonicalId| {
                crate::port_tick::read_pool(ctx, endpoint, good)
            };
            match crate::port::tv::hopper_load_with(w, rig.tv, pool_unit.as_ref(), &read) {
                Ok((kg, _)) => stalled.push(kg > 0),
                Err(_) => {
                    return DispatchRead::Unreadable {
                        what: format!("rig {}'s hopper", rig.n),
                    };
                }
            }
        } else {
            stalled.push(false);
        }
    }
    let ledger = match w
        .module_state::<crate::port::families::PortHandles>()
        .map(|handles| handles.families.dispatch.clone())
    {
        Some(handle) => match dispatch_at(
            ctx,
            &handle,
            &DispatchKey {
                site: entry.clone(),
            },
        ) {
            Ok(ledger) => ledger,
            Err(()) => {
                return DispatchRead::Unreadable {
                    what: "the delivery ledger".to_owned(),
                };
            }
        },
        None => {
            return DispatchRead::Unreadable {
                what: "the delivery ledger".to_owned(),
            };
        }
    };
    if crate::port_tick::read_pool(ctx, EndpointRef(hold), &yard.ore).is_err() {
        return DispatchRead::Unreadable {
            what: "the yard hold".to_owned(),
        };
    }
    let storefront = {
        let view = YardView {
            entry: entry.clone(),
            site,
            def: yard,
            map,
            row,
            house: house.clone(),
        };
        house_mill(w, &view).map(|(front, _)| front)
    };
    let front_ok = storefront.is_some_and(|front| {
        crate::port_tick::read_pool(ctx, EndpointRef(front), &yard.ore).is_ok()
    });
    if !front_ok {
        return DispatchRead::Unreadable {
            what: "the storefront".to_owned(),
        };
    }
    let open = matches!(opening_line(w, entry), SiteOpening::Open { .. });
    let view = YardView {
        entry: entry.clone(),
        site,
        def: yard,
        map,
        row,
        house: house.clone(),
    };
    let owed_now = owed(reads, &view, now);
    // 3. No instrument (unreachable: a pinned constant) once the seeds landed.
    let seeds_landed = !matches!(owed_now, Ok(Some(Owed::Seeds)) | Err(()));
    if open && seeds_landed && crate::economy::licence_instrument().is_none() {
        return DispatchRead::NoInstrument;
    }
    // 4. The licence recorded and its account not active.
    if row.licence.is_some() {
        let active = crate::economy::licence_instrument().is_some_and(|instrument| {
            mush_industry::licence::licence_accounts(w, &instrument, &house)
                == Some(mush_economy::AccountStatus::Active)
        });
        if !active {
            return DispatchRead::LicenceInactive;
        }
    }
    // 5. The first stopped rig, slot order.
    let store = w.module_state::<TacsimStore>();
    for (rig, stalled) in line_rigs.iter().zip(stalled.iter()) {
        let (n, mark) = (rig.n, rig.mark.clone());
        let ended = rig
            .record
            .as_ref()
            .and_then(|record| record.plan.as_ref())
            .and_then(|plan| match plan.state {
                FillState::Ended { why, .. } => Some(why),
                FillState::Live | FillState::Parking { .. } => None,
            });
        let parked_idle = rig.record.as_ref().is_some_and(|record| {
            record.act.is_none()
                && record.run.is_none()
                && record.plan.is_none()
                && matches!(
                    crate::surface::act::where_is(w, rig.tv),
                    crate::surface::act::Where::Ground { .. }
                )
        }) && store
            .and_then(|store| store.tv_rec(rig.tv))
            .is_some_and(|state| {
                matches!(state, crate::surface::excursion::TvRecState::Present(rec)
                    if rec.state == crate::surface::TvState::Parked)
            });
        // A stranded plan's rig staff have already brought in (5a's remedy followed: on the pad,
        // recovered into the bay) is no longer stranded: its ended plan still holds it (the re-arm
        // never follows `Stranded`), so it reads 5b's line and remedy — clear (strike) its plan
        // (the fixer round's correctness/F5).
        let in_bay = crate::surface::act::where_is(w, rig.tv)
            == (crate::surface::act::Where::Bay {
                hull: ObjId(hull.hull),
            });
        match ended {
            Some(why @ PlanEnd::Stranded) if in_bay => {
                return DispatchRead::Stopped { n, mark, why };
            }
            Some(PlanEnd::Stranded) => return DispatchRead::Stranded { n, mark },
            _ if parked_idle => return DispatchRead::Stranded { n, mark },
            Some(
                why @ (PlanEnd::Licence { .. }
                | PlanEnd::Claimed
                | PlanEnd::Lease { .. }
                | PlanEnd::Dark
                | PlanEnd::ChartFull),
            ) => return DispatchRead::Stopped { n, mark, why },
            Some(PlanEnd::Cleared) => return DispatchRead::Held { n, mark },
            _ if *stalled => return DispatchRead::Stalled { n, mark },
            Some(PlanEnd::WorkedOut | PlanEnd::HoldFull | PlanEnd::Bound) | None => {}
        }
    }
    // 6. The site not open.
    if !open {
        return DispatchRead::NotOpen {
            rigs,
            hull: hull_name,
        };
    }
    // 7. The clerk's items owed (an unreadable owed read reads as waiting).
    if !matches!(owed_now, Ok(None)) {
        return DispatchRead::Owed;
    }
    let total_t = ledger.as_ref().map_or(0, |ledger| ledger.delivered_t);
    let today = crate::port::mill::ic_day(now);
    let day_t = ledger
        .as_ref()
        .filter(|ledger| ledger.day == today)
        .map_or(0, |ledger| ledger.day_t);
    let records: Vec<&crate::surface::excursion::RigRecord> = line_rigs
        .iter()
        .filter_map(|rig| rig.record.as_ref())
        .collect();
    let out = u64::try_from(
        records
            .iter()
            .filter(|record| record.excursion.is_some())
            .count(),
    )
    .unwrap_or(u64::MAX);
    let working = u64::try_from(
        records
            .iter()
            .filter(|record| record.live_plan_for(dispatcher).is_some())
            .count(),
    )
    .unwrap_or(u64::MAX);
    let filed = records.iter().fold(0_u64, |sum, record| {
        sum.saturating_add(u64::from(record.plans))
    });
    // 8. Every rig worked out and never re-armed.
    let all_worked_out = out == 0
        && working == 0
        && line_rigs.len() == usize::try_from(yard.rigs).unwrap_or(usize::MAX)
        && line_rigs.iter().all(|rig| {
            rig.record.as_ref().is_some_and(|record| {
                let worked = record.plan.as_ref().is_some_and(|plan| {
                    matches!(
                        plan.state,
                        FillState::Ended {
                            why: PlanEnd::WorkedOut,
                            ..
                        }
                    )
                });
                let cycled = record
                    .plan
                    .as_ref()
                    .is_some_and(|plan| plan.tally.cycles >= 1);
                let fair = cycled
                    && family_of(yard).is_some_and(|family| {
                        // A ground that is cold or dark reads as ore left: never claim worked
                        // out on a read that did not happen.
                        !matches!(
                            fair_left_now(ctx, dispatcher, entry, family, &yard.blocks),
                            FairRead::Read(false)
                        )
                    });
                worked
                    && file_due(record.plan.as_ref(), yard.rearm_ic_secs, now, fair)
                        == FileDue::Never
            })
        });
    if all_worked_out {
        return DispatchRead::WorkedOut {
            rigs,
            hull: hull_name,
            ore: yard.ore.clone(),
            total_t,
        };
    }
    // 9. Anything filed or delivered.
    if filed > 0 || ledger.is_some() {
        return DispatchRead::Working {
            out,
            rigs,
            working,
            filed,
            day_t,
            ore: yard.ore.clone(),
            total_t,
        };
    }
    // 10. Nothing yet.
    DispatchRead::Nothing {
        rigs,
        hull: hull_name,
    }
}

/// The yard's ore family.
fn family_of(yard: &YardDef) -> Option<OreFamily> {
    crate::surface::geology::family_of_word(yard.ore.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap_or_else(|_| panic!("{text} is an id"))
    }

    /// **Doc 20 cut SM.12b**: a delivery fills the storefront to a day of milling, never past it,
    /// and never more than the hold holds — saturating.
    #[test]
    fn sm12b_delivery_fills_to_a_day_of_milling() {
        assert_eq!(deliverable(30, 0, 40), 30);
        assert_eq!(deliverable(86, 25, 40), 15);
        assert_eq!(deliverable(86, 40, 40), 0);
        assert_eq!(deliverable(86, 80, 40), 0);
        assert_eq!(deliverable(0, 0, 40), 0);
        assert_eq!(deliverable(u64::MAX, 0, u64::MAX), u64::MAX);
        assert_eq!(deliverable(5, u64::MAX, 40), 0);
    }

    fn ledger() -> DispatchLedger {
        DispatchLedger {
            site: cid("occator_port"),
            house: 7,
            dispatcher: ObjId(99),
            good: cid("ice_ore"),
            hold: VaultId(1),
            storefront: VaultId(2),
            deliveries: 2,
            delivered_t: 70,
            day: crate::port::mill::ic_day(90_000),
            day_t: 30,
            recent: vec![
                Delivery {
                    ordinal: 1,
                    at_ic: 10,
                    t: 40,
                },
                Delivery {
                    ordinal: 2,
                    at_ic: 90_000,
                    t: 30,
                },
            ],
        }
    }

    /// **Doc 20 cut SM.12b**: a dispatch ledger checks its tail — the count, the ordinals, the
    /// tonnes, the day and its tally; the key text is the site, strictly.
    #[test]
    fn sm12b_a_dispatch_ledger_checks_its_tail() {
        assert_eq!(ledger().check(), Ok(()));
        let broken = |edit: &dyn Fn(&mut DispatchLedger)| {
            let mut l = ledger();
            edit(&mut l);
            l.check().is_err()
        };
        assert!(broken(&|l| l.deliveries = 0));
        assert!(broken(&|l| l.recent.clear()));
        assert!(broken(&|l| l.deliveries = 3));
        assert!(broken(&|l| l.recent[0].ordinal = 0));
        assert!(broken(&|l| l.recent[1].t = 0));
        assert!(broken(&|l| l.delivered_t = 60));
        assert!(broken(&|l| l.day += 1));
        assert!(broken(&|l| l.day_t = 29));
        assert!(broken(&|l| l.day_t = 71));
        assert!(broken(&|l| {
            l.recent = (1..=17)
                .map(|ordinal| Delivery {
                    ordinal,
                    at_ic: 90_000,
                    t: 1,
                })
                .collect();
            l.deliveries = 17;
            l.delivered_t = 17;
            l.day_t = 17;
        }));
        // The wire is strict.
        let mut json = serde_json::to_value(ledger()).unwrap_or_default();
        json["x"] = json!(1);
        assert!(serde_json::from_value::<DispatchLedger>(json).is_err());
        // The key.
        let key = DispatchKey {
            site: cid("occator_port"),
        };
        assert_eq!(key.key_text(), "occator_port");
        assert_eq!(DispatchKey::parse("occator_port"), Some(key));
        assert_eq!(DispatchKey::parse("Occator_Port"), None);
        assert_eq!(DispatchKey::parse(""), None);
    }

    fn plan(state: FillState, cycles: u16) -> FillPlan {
        FillPlan {
            no: 1,
            author: ObjId(99),
            hull: ObjId(50),
            slot: 1,
            site: cid("occator_port"),
            family: OreFamily::Volatiles,
            filed_ic: 0,
            deadline_ic: crate::surface::plan::PLAN_BOUND_IC_SECS,
            list: vec!["F".to_owned(), "G".to_owned()],
            steps: vec![crate::surface::plan::PlanStep::Block { letter: 'F' }],
            cursor: 1,
            passed: Vec::new(),
            state,
            seq: 1,
            tally: crate::surface::plan::PlanTally {
                tonnes: 0,
                cycles,
                trips: 0,
            },
            notes: Vec::new(),
        }
    }

    /// **Doc 20 cut SM.12b (O-14)**: the re-arm rule, every end — `HoldFull` and `Bound` after the
    /// interval (its edge pinned), `WorkedOut` only with fair ore left and a landed cycle, never
    /// after a clear, a right, a stranding, a full chart or the dark registry; a running plan
    /// never; no plan now.
    #[test]
    fn sm12b_file_due_rearms_only_on_its_rule() {
        let rearm = 3_600;
        let ended = |why: PlanEnd| FillState::Ended { why, at_ic: 1_000 };
        assert_eq!(file_due(None, rearm, 0, false), FileDue::Now);
        assert_eq!(
            file_due(Some(&plan(FillState::Live, 0)), rearm, 9_999_999, true),
            FileDue::Never
        );
        assert_eq!(
            file_due(
                Some(&plan(
                    FillState::Parking {
                        why: PlanEnd::HoldFull
                    },
                    3
                )),
                rearm,
                9_999_999,
                true
            ),
            FileDue::Never
        );
        for why in [PlanEnd::HoldFull, PlanEnd::Bound] {
            let p = plan(ended(why), 0);
            assert_eq!(
                file_due(Some(&p), rearm, 1_000 + rearm - 1, false),
                FileDue::At(1_000 + rearm),
                "{why:?} one second short"
            );
            assert_eq!(
                file_due(Some(&p), rearm, 1_000 + rearm, false),
                FileDue::Now,
                "{why:?} on the edge"
            );
        }
        for (fair, cycles, want) in [
            (false, 0, FileDue::Never),
            (false, 1, FileDue::Never),
            (true, 0, FileDue::Never),
            (true, 1, FileDue::Now),
        ] {
            assert_eq!(
                file_due(
                    Some(&plan(ended(PlanEnd::WorkedOut), cycles)),
                    rearm,
                    1_000 + rearm,
                    fair
                ),
                want,
                "worked out, fair {fair}, cycles {cycles}"
            );
        }
        assert_eq!(
            file_due(
                Some(&plan(ended(PlanEnd::WorkedOut), 1)),
                rearm,
                1_000 + rearm - 1,
                true
            ),
            FileDue::At(1_000 + rearm)
        );
        for why in [
            PlanEnd::Cleared,
            PlanEnd::Licence {
                why: crate::surface::excursion::LicenceEnd::Suspended,
            },
            PlanEnd::Claimed,
            PlanEnd::Lease { forfeit: false },
            PlanEnd::Dark,
            PlanEnd::ChartFull,
            PlanEnd::Stranded,
        ] {
            assert_eq!(
                file_due(Some(&plan(ended(why), 5)), rearm, i64::MAX, true),
                FileDue::Never,
                "{why:?}"
            );
        }
        // Saturating: an interval past the end of time never wraps.
        assert_eq!(
            file_due(
                Some(&plan(
                    FillState::Ended {
                        why: PlanEnd::HoldFull,
                        at_ic: i64::MAX - 1
                    },
                    0
                )),
                rearm,
                0,
                false
            ),
            FileDue::At(i64::MAX)
        );
    }

    /// **Doc 20 cut SM.12b**: `fair_left` reads as `decide`'s Block step does — a fair pocket
    /// that is worked out does not count, one fair open pocket does, lean ore does not, and a
    /// pocket outside the blocks is ignored.
    #[test]
    fn sm12b_fair_left_reads_as_the_block_step_does() {
        let in_f = crate::surface::address::pocket_of_block_index('F', 7)
            .unwrap_or_else(|| panic!("F has pockets"));
        let in_a = crate::surface::address::pocket_of_block_index('A', 7)
            .unwrap_or_else(|| panic!("A has pockets"));
        let one =
            |at: (i64, i64), word: Word| move |pocket: (i64, i64)| (pocket == at).then_some(word);
        // Fair but worked out: false.
        assert!(!fair_left(
            one(in_f, Word::Fair),
            |p| p == in_f,
            &['F', 'G']
        ));
        // Fair and open: true; rich too.
        assert!(fair_left(one(in_f, Word::Fair), |_| false, &['F', 'G']));
        assert!(fair_left(one(in_f, Word::Rich), |_| false, &['F']));
        // Lean only: false.
        assert!(!fair_left(one(in_f, Word::Lean), |_| false, &['F', 'G']));
        // A pocket outside the blocks is ignored.
        assert!(!fair_left(one(in_a, Word::Rich), |_| false, &['F', 'G']));
        // Nothing read: false.
        assert!(!fair_left(|_| None, |_| false, &['F', 'G']));
    }

    /// **Doc 20 cut SM.12b (O-18)**: every Dispatch line text, whole, with its state.
    #[test]
    fn sm12b_the_dispatch_line_texts() {
        let entry = cid("occator_port");
        let org = cid("occator");
        let line = |read: DispatchRead| dispatch_line_text(&entry, &org, &read);
        let head = "occator_port dispatch occator: ";
        let hull = "Occator Yard".to_owned();
        assert_eq!(
            line(DispatchRead::Missing),
            (
                LineState::Missing,
                format!("{head}the yard is not laid up.")
            )
        );
        for what in [
            "rig 1's record",
            "rig 2's hopper",
            "the delivery ledger",
            "the yard hold",
            "the storefront",
        ] {
            assert_eq!(
                line(DispatchRead::Unreadable {
                    what: what.to_owned()
                }),
                (LineState::Flagged, format!("{head}{what} cannot be read."))
            );
        }
        assert_eq!(
            line(DispatchRead::NoInstrument),
            (
                LineState::Flagged,
                format!("{head}no licence instrument is set, so the house cannot be licensed.")
            )
        );
        assert_eq!(
            line(DispatchRead::LicenceInactive),
            (
                LineState::Flagged,
                format!("{head}its licence account is not active.")
            )
        );
        assert_eq!(
            line(DispatchRead::Stranded {
                n: 1,
                mark: "OWP-YT-01".to_owned()
            }),
            (
                LineState::Flagged,
                format!(
                    "{head}rig 1 (OWP-YT-01) is stranded on the ground. Staff bring it in by \
                     placing it on the pad, then recover it as the dispatcher."
                )
            )
        );
        assert_eq!(
            line(DispatchRead::Stopped {
                n: 2,
                mark: "OWP-YT-02".to_owned(),
                why: PlanEnd::Claimed
            }),
            (
                LineState::Flagged,
                format!(
                    "{head}rig 2 (OWP-YT-02) stopped: under claim. Staff clear its plan to start \
                     it again."
                )
            )
        );
        // A stranded rig staff brought back into the bay (the fixer round's correctness/F5):
        // its ended plan still holds it, so 5b names the strike.
        assert_eq!(
            line(DispatchRead::Stopped {
                n: 1,
                mark: "OWP-YT-01".to_owned(),
                why: PlanEnd::Stranded
            }),
            (
                LineState::Flagged,
                format!(
                    "{head}rig 1 (OWP-YT-01) stopped: stranded. Staff clear its plan to start it \
                     again."
                )
            )
        );
        assert_eq!(
            line(DispatchRead::Held {
                n: 1,
                mark: "OWP-YT-01".to_owned()
            }),
            (
                LineState::Flagged,
                format!(
                    "{head}rig 1 (OWP-YT-01) is held by staff; strike its plan to start it again."
                )
            )
        );
        assert_eq!(
            line(DispatchRead::Stalled {
                n: 1,
                mark: "OWP-YT-01".to_owned()
            }),
            (
                LineState::Flagged,
                format!(
                    "{head}rig 1 (OWP-YT-01) holds ore its last unload left in the hopper. Staff \
                     unload it as the dispatcher (+tv/1 unload go) to start it again."
                )
            )
        );
        assert_eq!(
            line(DispatchRead::NotOpen {
                rigs: 2,
                hull: hull.clone()
            }),
            (
                LineState::Ready,
                format!(
                    "{head}2 rigs in the Occator Yard's bay; the site is not open, so nothing is \
                     filed."
                )
            )
        );
        assert_eq!(
            line(DispatchRead::Owed),
            (
                LineState::Pending,
                format!("{head}waiting on the Registry's seeds, licence and survey.")
            )
        );
        assert_eq!(
            line(DispatchRead::WorkedOut {
                rigs: 2,
                hull: hull.clone(),
                ore: cid("ice_ore"),
                total_t: 172
            }),
            (
                LineState::Ready,
                format!(
                    "{head}its blocks read worked out in ice ore; 2 rigs parked in the Occator \
                     Yard's bay, 172 t delivered in all."
                )
            )
        );
        assert_eq!(
            line(DispatchRead::Working {
                out: 0,
                rigs: 2,
                working: 1,
                filed: 1,
                day_t: 0,
                ore: cid("ice_ore"),
                total_t: 0
            }),
            (
                LineState::Ready,
                format!(
                    "{head}0 of 2 out, 1 working a plan, 1 plan filed in all; 0 t of ice ore \
                     delivered to its storefront today, 0 t in all."
                )
            )
        );
        assert_eq!(
            line(DispatchRead::Working {
                out: 2,
                rigs: 2,
                working: 2,
                filed: 5,
                day_t: 30,
                ore: cid("ice_ore"),
                total_t: 116
            })
            .1,
            format!(
                "{head}2 of 2 out, 2 working a plan, 5 plans filed in all; 30 t of ice ore \
                 delivered to its storefront today, 116 t in all."
            )
        );
        assert_eq!(
            line(DispatchRead::Nothing { rigs: 1, hull }),
            (
                LineState::Ready,
                format!("{head}1 rig in the Occator Yard's bay; nothing filed yet.")
            )
        );
    }

    /// **Doc 20 cut SM.12b**: the house views a yard row gives — slot order from `n + 1`, the
    /// dispatcher their pilot, the yard hull their hull, never a stored lease.
    #[test]
    fn sm12b_the_yard_queries_read_the_map() {
        let row = YardRow {
            hulls: vec![crate::port::map::HullRow {
                n: 0,
                mark: "OWP-YD-01".to_owned(),
                hull: 50,
                rooms: BTreeMap::new(),
                exits: Vec::new(),
            }],
            rigs: vec![
                crate::port::map::RigRow {
                    n: 1,
                    mark: "OWP-YT-02".to_owned(),
                    tv: 61,
                    rooms: BTreeMap::new(),
                    vault: None,
                },
                crate::port::map::RigRow {
                    n: 0,
                    mark: "OWP-YT-01".to_owned(),
                    tv: 60,
                    rooms: BTreeMap::new(),
                    vault: None,
                },
            ],
            dispatcher: Some(70),
            hold: Some(5),
            licence: None,
        };
        let views = house_views(&row, ObjId(70));
        assert_eq!(
            views
                .iter()
                .map(|view| (
                    view.slot,
                    view.tv,
                    view.mark.as_str(),
                    view.hull,
                    view.pilot
                ))
                .collect::<Vec<_>>(),
            vec![
                (1, ObjId(60), "OWP-YT-01", ObjId(50), ObjId(70)),
                (2, ObjId(61), "OWP-YT-02", ObjId(50), ObjId(70)),
            ]
        );
        assert!(views.iter().all(TvLease::is_house));
        assert!(views.iter().all(|view| view.check().is_err()));
        // No hull recorded: no rigs to operate.
        let bare = YardRow {
            hulls: Vec::new(),
            ..row.clone()
        };
        assert!(house_views(&bare, ObjId(70)).is_empty());
        // A row from before the cut encodes with no yard key; an empty yard row encodes empty.
        let site = SiteRow::default();
        let json = serde_json::to_value(&site).unwrap_or_default();
        assert!(json.get("yard").is_none(), "{json}");
        assert_eq!(
            serde_json::to_value(YardRow::default()).unwrap_or_default(),
            json!({})
        );
        // Held letters: the house's held blocks, letter order.
        let mut def = SiteDef {
            name: "Occator Rim".to_owned(),
            issuer: cid("freeport_registry"),
            zone: crate::port::def::Zone::Core,
            tenure: crate::port::def::SiteTenureDef {
                default: Tenure::Open,
            },
            blocks: std::collections::BTreeMap::new(),
            papers: std::collections::BTreeMap::new(),
            yard: None,
        };
        for letter in ["K", "F", "J", "G"] {
            def.blocks.insert(
                letter.to_owned(),
                crate::port::def::BlockDef {
                    tenure: Some(Tenure::Held),
                    holder: Some(cid("occator")),
                    ..crate::port::def::BlockDef::default()
                },
            );
        }
        def.blocks.insert(
            "A".to_owned(),
            crate::port::def::BlockDef {
                tenure: Some(Tenure::Held),
                holder: Some(cid("pallas_syndicate")),
                ..crate::port::def::BlockDef::default()
            },
        );
        assert_eq!(
            held_letters(&def, &cid("occator")),
            vec!['F', 'G', 'J', 'K']
        );
        assert_eq!(variant_word(&PlanEnd::HoldFull), "HoldFull");
        assert_eq!(variant_word(&PlanEnd::Lease { forfeit: true }), "Lease");
    }
}
