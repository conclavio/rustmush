//! The market's own seams: the catalog, vendors, listings and NPC market makers
//! (E6c, B3/B5/B8).
//!
//! Everything that moves money moves it through E4 under an E4 authority — the
//! `offer_contract` / `accept_contract` / `accept_contract_as` / `cancel_contract`
//! seams and `create_system_obligation` for a registered faucet's drip. **No
//! `@contract`, `@obligation`, `@asset`, `@vault` or `@org` command is ever
//! dispatched from a handler in this crate**, and there is no journal write anywhere
//! in it.

use mush_core::{Ctx, ObjId, World};
use mush_economy::{
    AccountId, AccountStatus, Amount, CatalogRef, EconomicClock, EconomicDeadline, EconomyStore,
    InstrumentId, VaultId, VaultStatus,
};
use mush_org::{OrgId, PrincipalId};
use smol_str::SmolStr;

use crate::ids::{ListingId, MarketId, MarketSourceId, VendorId};
use crate::record::{
    CatalogEntry, Legality, Listing, MAX_CATALOG_ENTRIES_PER_PACK, MAX_LISTINGS,
    MAX_LISTINGS_PER_VENDOR, MAX_SOURCES, MAX_VENDORS, MarketSource, MarketVisibility,
    Representation, Supply, Vendor, VendorStatus, check_entry,
};
use crate::store::{
    MARKET_PAGE_MAX, MARKET_PAGE_VISIT, MarketStore, del_entry, del_listing, is_live_object,
    persist_meta, put_entry, put_listing, put_source, put_vendor,
};

/// The one refusal a listing the actor may not use ever gets — identical for
/// nonexistent and unviewable (08a decision 13).
pub const NO_SUCH_LISTING: &str = "No such listing.";
/// The same, for a quote.
pub const NO_SUCH_QUOTE: &str = "No such quote.";
/// The same, for a vendor.
pub const NO_SUCH_VENDOR: &str = "No such vendor.";
/// The same, for a market maker.
pub const NO_SUCH_SOURCE: &str = "No such market source.";
/// The same, for a catalogue entry.
pub const NO_SUCH_ENTRY: &str = "No such catalogue entry.";

/// The longest a listing stands before it lapses, in seconds, and the ceiling on a
/// caller-supplied window.
pub const MAX_LISTING_TTL_SECS: i64 = 90 * 86_400;
/// The default listing window.
pub const DEFAULT_LISTING_TTL_SECS: i64 = 14 * 86_400;

/// Why a market act was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarketError {
    /// Nothing the actor may act on — the existence-hiding answer, and the **only**
    /// answer a nonexistent and an unviewable record both get.
    NoStanding(&'static str),
    /// A shaped refusal that says something true about the command itself.
    Bad(String),
    /// The economy refused the offer or the accept.
    Contract(mush_economy::ContractError),
    /// The economy refused a post.
    Economy(mush_economy::EconomyError),
}

impl MarketError {
    /// A `Bad` from anything string-shaped.
    #[must_use]
    pub fn bad(msg: impl Into<String>) -> MarketError {
        MarketError::Bad(msg.into())
    }
}

/// Render a [`MarketError`] under the existence-hiding discipline, counting every
/// existence probe through the economy's own C16 refusal telemetry (E6b, D8): the
/// market mints four brand-new enumerable id spaces, and a refusal built straight
/// into a `CmdError` would never reach the tally staff read the shape of a walk from.
#[must_use]
pub fn refuse_market(ctx: &mut Ctx<'_>, e: &MarketError) -> mush_core::CmdError {
    match e {
        MarketError::NoStanding(why) => {
            mush_economy::note_refusal(ctx, true);
            mush_core::CmdError::msg((*why).to_string())
        }
        MarketError::Bad(why) => {
            mush_economy::note_refusal(ctx, false);
            mush_core::CmdError::msg(why.clone())
        }
        MarketError::Contract(e) => mush_economy::refuse_contract_offer(ctx, e, Vec::new()),
        MarketError::Economy(e) => {
            mush_economy::refuse(ctx, mush_economy::NamedResource::Asset(ObjId(0)), &[], e)
        }
    }
}

// ---- small world questions -------------------------------------------------

/// Staff, by the economy's own definition — one rule across the E-series.
#[must_use]
pub fn is_staff(w: &World, who: ObjId) -> bool {
    mush_core::perm::has_power(w, who, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(w, who, mush_core::Role::Royalty)
}

/// A principal's own `Active` account in one instrument — the derivation E6b's fence
/// uses, so nobody ever names a wallet.
///
/// A person's is their **unscoped** one; an organization's is always scoped, so the
/// rule is "the principal's own live account in this instrument, lowest id first".
#[must_use]
pub fn principal_account(
    ctx: &Ctx<'_>,
    instrument: &InstrumentId,
    who: &PrincipalId,
) -> Option<AccountId> {
    let personal = matches!(who, PrincipalId::Object(_));
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(who)
        .find(|a| {
            a.instrument == *instrument
                && (!personal || a.scope.is_none())
                && a.status == AccountStatus::Active
        })
        .map(|a| a.id)
}

/// The instrument one account holds.
#[must_use]
pub fn account_instrument(ctx: &Ctx<'_>, account: AccountId) -> Option<InstrumentId> {
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .account(account)
        .map(|a| a.instrument.clone())
}

/// The **hidden** selling organizations, for the projection's public test.
///
/// **Derived from the records the projection actually folds** — every listing's seller
/// and every trade row's seller — and not from the vendor table (independent verify
/// fold, D1).
///
/// It used to compose the set from `store.vendors()` alone, and the vendor table is not
/// the same set. `store::load` is deliberately **total**: a corrupt `market:vendor:{id}`
/// is skipped and the world boots, while that org's `market:listing:{id}` documents
/// load fine — a `Listing` denormalizes `seller: PrincipalId::Organization(org)` and its
/// wire never consults the vendor table. One skipped vendor document therefore dropped
/// its organization out of `hidden_orgs`, `is_public_seller` answered `true`, and a
/// hidden shop's price and quantity were summed into `band`/`depth` and rendered to a
/// stranger by `+market/depth` — while `may_view_listing` → `list_org` still hid the
/// very same listing from `+market/list`. The documented fail-closed in
/// [`is_hidden_org`] never fired because nothing asked it. Any vendor-row loss did it:
/// a `load_one` cap skip, a `Del` that outran its listings. Asking the records
/// themselves means every organization that *can* appear is asked, so the fail-closed
/// fires; the walk is bounded by `MAX_LISTINGS` + `MAX_QUOTES`, which are already the
/// load caps.
#[must_use]
pub fn hidden_seller_orgs(ctx: &Ctx<'_>) -> Vec<OrgId> {
    let Some(store) = ctx.w.module_state::<MarketStore>() else {
        return Vec::new();
    };
    let sellers: std::collections::BTreeSet<OrgId> = store
        .listings
        .values()
        .map(|l| &l.seller)
        .chain(
            store
                .quotes
                .values()
                .filter(|q| q.is_trade())
                .map(|q| &q.seller),
        )
        .filter_map(|seller| match seller {
            PrincipalId::Organization(org) => Some(*org),
            PrincipalId::Object(_) | PrincipalId::System(_) => None,
        })
        .collect();
    sellers
        .into_iter()
        .filter(|org| is_hidden_org(ctx.w, *org))
        .collect()
}

/// Whether an organization is hidden. Fails **closed**: an org the registry cannot
/// answer for is treated as hidden, so a missing record can never widen the public
/// set.
#[must_use]
pub fn is_hidden_org(w: &World, org: OrgId) -> bool {
    w.module_state::<mush_org::OrgStore>()
        .and_then(|st| st.state().org(org).map(|o| o.hidden))
        .unwrap_or(true)
}

/// Whether an organization may **agree to new trade** — register as a vendor, publish a
/// listing, be quoted, sell (independent verify fold, M4).
///
/// DL-06's `OrgStatus` reached nothing in the market: no file in this crate mentioned
/// `Suspended`, `Dissolving` or `Closed`, so a closed organization went on quoting,
/// settling and re-registering, and `@org/status` — B10's only staff verb for shutting a
/// shop down — had no effect here at all. The economy's own gates do not catch it,
/// because `scope_coherent` and `credit_allowed` bite on a *scoped* account and a
/// vendor's wallet is the org's own unscoped one.
///
/// `Dissolving` **may settle what it already agreed to** and may agree to nothing more,
/// which is what "a close workflow is in progress" means; live contracts are the
/// economy's and are never touched from here. Fails **closed**, the same direction
/// [`is_hidden_org`] fails and `Vendor::is_trading` already answers.
#[must_use]
pub fn org_may_trade(w: &World, org: OrgId) -> bool {
    w.module_state::<mush_org::OrgStore>()
        .and_then(|st| st.state().org(org).map(|o| o.status))
        .is_some_and(|s| matches!(s, mush_org::OrgStatus::Active))
}

/// Whether an organization has reached the **terminal** `Closed` status — the one that
/// never re-opens, and therefore the one a boot reconcile may retire a vendor for.
#[must_use]
pub fn org_is_closed(w: &World, org: OrgId) -> bool {
    w.module_state::<mush_org::OrgStore>()
        .and_then(|st| st.state().org(org).map(|o| o.status))
        .is_some_and(|s| matches!(s, mush_org::OrgStatus::Closed))
}

// ---- B3: the catalog and its content packs ---------------------------------

/// What loading one pack did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackLoad {
    /// The pack's name.
    pub pack: SmolStr,
    /// How many entries it now holds.
    pub entries: usize,
    /// How many were newly added.
    pub added: usize,
    /// How many were replaced by a newer definition version.
    pub updated: usize,
    /// How many the reload dropped (they are no longer in the pack).
    pub dropped: usize,
}

/// **Load one content pack** (B3, answers 9–10).
///
/// Idempotent: loading the same pack twice adds nothing and drops nothing. A pack
/// whose entries carry a higher `definition_version` **versions** the entries in
/// place, which is what a quote's stale-version refusal is about. Every entry that
/// the pack no longer defines is removed.
///
/// **A bad entry fails the pack, not the server** (B3): validation runs over the
/// whole list before a single document is written, and the refusal names the entry.
/// The caller — `@market/catalog load` — names the file.
///
/// # Errors
///
/// [`MarketError::Bad`] naming the first entry that failed and why.
pub fn load_pack(
    ctx: &mut Ctx<'_>,
    pack: &str,
    entries: Vec<CatalogEntry>,
) -> Result<PackLoad, MarketError> {
    let pack = SmolStr::new(pack.trim());
    if entries.len() > MAX_CATALOG_ENTRIES_PER_PACK {
        return Err(MarketError::bad(format!(
            "pack '{pack}' declares {} entries; this server allows {MAX_CATALOG_ENTRIES_PER_PACK} \
             per pack.",
            entries.len()
        )));
    }
    // Validate the WHOLE pack first: nothing is written until every entry is good,
    // so a bad entry costs the pack and leaves the catalog exactly as it was.
    let mut seen: std::collections::BTreeSet<SmolStr> = std::collections::BTreeSet::new();
    for e in &entries {
        if e.pack != pack {
            return Err(MarketError::bad(format!(
                "entry '{}' claims pack '{}' but was loaded as '{pack}'.",
                e.catalog.code, e.pack
            )));
        }
        check_entry(e).map_err(|why| {
            MarketError::bad(format!("entry '{}' is malformed: {why}.", e.catalog.code))
        })?;
        if !seen.insert(e.catalog.code.clone()) {
            return Err(MarketError::bad(format!(
                "entry '{}' is declared twice in pack '{pack}'.",
                e.catalog.code
            )));
        }
        // One live entry per `CatalogRef` across every pack: two packs defining the
        // same code would make "what is this" a question about load order.
        let clash = ctx.w.module_state::<MarketStore>().and_then(|s| {
            s.entry_of(&e.catalog)
                .filter(|other| other.pack != pack)
                .map(|other| other.pack.clone())
        });
        if let Some(other) = clash {
            return Err(MarketError::bad(format!(
                "entry '{}' is already defined by pack '{other}'.",
                e.catalog.code
            )));
        }
    }
    let previous: Vec<CatalogEntry> = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| s.entries_of_pack(pack.as_str()).cloned().collect())
        .unwrap_or_default();
    // The whole-store ceiling on **write** as well as on load (E6b independent verify
    // fold, D5's rule): a cap enforced only at boot lets a store grow past it
    // legitimately and then silently drops the overflow on the next restart. Counted
    // against what this pack would *add*, so a reload of a pack already in the
    // catalogue never trips it.
    let net_new = entries
        .iter()
        .filter(|e| !previous.iter().any(|p| p.catalog.code == e.catalog.code))
        .count();
    let held = ctx
        .w
        .module_state::<MarketStore>()
        .map_or(0, |s| s.entries().count());
    if held.saturating_add(net_new) > crate::record::MAX_CATALOG_ENTRIES {
        return Err(MarketError::bad(format!(
            "pack '{pack}' would take the catalogue past the {} entries this server holds.",
            crate::record::MAX_CATALOG_ENTRIES
        )));
    }
    let mut out = PackLoad {
        pack: pack.clone(),
        entries: entries.len(),
        ..PackLoad::default()
    };
    for e in &entries {
        match previous.iter().find(|p| p.catalog.code == e.catalog.code) {
            None => out.added = out.added.saturating_add(1),
            Some(p) if p != e => out.updated = out.updated.saturating_add(1),
            Some(_) => continue, // byte-identical: the reload is a no-op for it
        }
        put_entry(ctx, e.clone());
    }
    for p in &previous {
        if !seen.contains(&p.catalog.code) {
            out.dropped = out.dropped.saturating_add(1);
            del_entry(ctx, &p.pack, &p.catalog.code);
            // **A dropped entry takes its listings with it** (independent verify fold,
            // R6). `del_entry` touched no listing, and `projection` folds
            // `listings_for` without ever consulting the catalogue — so the orphaned
            // listings kept contributing band, depth and the seller's inventory count
            // for ever and `visible` kept rendering them, while `quote` refused
            // `NO_SUCH_LISTING`, `is_current` answered false and `deliver` logged "a
            // settled purchase names a catalogue entry that is gone". With nothing
            // sweeping them either (R3), the public projection permanently advertised
            // depth in a good that could not be bought. The vendor re-lists when the
            // entry comes back, which is the same trade `del_listing` already makes
            // everywhere else.
            let orphans: Vec<ListingId> = ctx
                .w
                .module_state::<MarketStore>()
                .map(|s| {
                    s.listings
                        .values()
                        .filter(|l| l.catalog == p.catalog)
                        .map(|l| l.id)
                        .take(MAX_LISTINGS)
                        .collect()
                })
                .unwrap_or_default();
            if !orphans.is_empty() {
                tracing::warn!(
                    pack = %p.pack,
                    code = %p.catalog.code,
                    listings = orphans.len(),
                    "market: a pack reload dropped an entry; its listings were withdrawn"
                );
            }
            for id in orphans {
                del_listing(ctx, id);
            }
        }
    }
    Ok(out)
}

// ---- B5: vendors -----------------------------------------------------------

/// **Register (or re-register) a vendor organization** (B5) — the neutral
/// generalization of E6b's `@sr/fence <org>=<vault>`.
///
/// The vendor's account is **derived** from the organization's own live account in
/// `instrument`, so nobody names a wallet: every escrow it places posts under that
/// account's own debit authority, and a vendor that has run out simply cannot accept.
/// That is the whole of "a vendor that runs out stops quoting" (M19, D-E6-15).
///
/// # Errors
///
/// [`MarketError`]; render it with [`refuse_market`].
pub fn register_vendor(
    ctx: &mut Ctx<'_>,
    instrument: &InstrumentId,
    org: OrgId,
    vault: VaultId,
    market: MarketId,
    name: SmolStr,
    buy_permille: u32,
) -> Result<VendorId, MarketError> {
    let principal = PrincipalId::Organization(org);
    // DL-06 reaches the market (independent verify fold, M4). `mush-org`'s own
    // `org_status` doc comment already promised this — "a suspended or dissolving
    // organization stops being registrable as a trading vendor (E6c B5/B10)" — and
    // nothing implemented it. Existence-hiding, because whether an organization exists
    // at all is not this refusal's to disclose.
    if !org_may_trade(ctx.w, org) {
        return Err(MarketError::NoStanding(NO_SUCH_VENDOR));
    }
    let account = principal_account(ctx, instrument, &principal)
        .ok_or(MarketError::NoStanding(NO_SUCH_VENDOR))?;
    // The vault must exist, be active, and belong to the same organization: a vendor
    // taking custody into somebody else's vault would hand the goods to a third party
    // under the buyer's nose.
    let ok = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().vault(vault).cloned())
        .is_some_and(|v| v.owner == principal && v.status == VaultStatus::Active);
    if !ok {
        return Err(MarketError::NoStanding(NO_SUCH_VENDOR));
    }
    if buy_permille > 1_000 {
        return Err(MarketError::bad(
            "A vendor's buy rate is a permille of the catalogue baseline, 0..=1000.",
        ));
    }
    if let Some(existing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor_of_org(org).cloned())
    {
        // E6b independent verify fold (M9), the same rule one layer up: **a vendor's
        // vault does not move under a live deal.** A frozen `TitleHeld`/`DepositInto`
        // names the vault by id and no re-pointing can re-open a frozen surface, so
        // the change is refused while anything still stands against it.
        if existing.vault != vault
            && ctx
                .w
                .module_state::<MarketStore>()
                .is_some_and(|s| s.listings_of_vendor(existing.id) > 0)
        {
            return Err(MarketError::bad(format!(
                "Vendor #{} still has listings standing against vault #{}; retire them first.",
                existing.id, existing.vault
            )));
        }
        let updated = Vendor {
            vault,
            account,
            market,
            name,
            buy_permille,
            status: VendorStatus::Active,
            revision: existing.revision.saturating_add(1),
            ..existing
        };
        put_vendor(ctx, updated);
        return Ok(existing.id);
    }
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.vendors().count() >= MAX_VENDORS)
    {
        return Err(MarketError::bad(
            "This server already registers as many vendors as it allows.",
        ));
    }
    let Some(id) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_vendor)
    else {
        return Err(MarketError::bad("The vendor-id allocator is exhausted."));
    };
    put_vendor(
        ctx,
        Vendor {
            id,
            org,
            market,
            account,
            vault,
            name,
            buy_permille,
            status: VendorStatus::Active,
            revision: 1,
        },
    );
    persist_meta(ctx);
    Ok(id)
}

/// **Retire a vendor** (B5, B10): it stops quoting immediately and every listing it
/// holds is withdrawn. Its **live contracts settle through E4 untouched** — retiring
/// a shop is not tearing up its agreements.
///
/// # Errors
///
/// [`MarketError`]; render it with [`refuse_market`].
pub fn retire_vendor(ctx: &mut Ctx<'_>, id: VendorId) -> Result<usize, MarketError> {
    let Some(v) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(id).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_VENDOR));
    };
    // **Every** listing, not the first 64 (independent verify fold, R8): `.take(
    // MAX_LISTINGS_PER_VENDOR)` was true of a store the cap had always held, and a
    // store carrying more — an operator lowering the constant, a hand-repaired
    // document, a future migration — retired the shop and left the surplus live,
    // publishing band and depth under a vendor that no longer exists (`projection`
    // never asks about the vendor, so nothing else would have caught it). Bounded by
    // `MAX_LISTINGS`, which is the whole store.
    let listings: Vec<ListingId> = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| {
            s.by_vendor
                .get(&id)
                .into_iter()
                .flatten()
                .copied()
                .take(MAX_LISTINGS)
                .collect()
        })
        .unwrap_or_default();
    let n = listings.len();
    for l in listings {
        del_listing(ctx, l);
    }
    put_vendor(
        ctx,
        Vendor {
            status: VendorStatus::Retired,
            revision: v.revision.saturating_add(1),
            ..v
        },
    );
    Ok(n)
}

/// **Fund a vendor or a maker from a registered faucet** (B8, doc 10 7A).
///
/// The **only** route by which value enters a vendor or a maker other than a sale:
/// a `create_system_obligation` drip from a registered faucet, under the `[economy]`
/// mint cap, re-checked live at every settlement by the economy's own sweep. There is
/// no other mint here — a maker is never a `System` party (D-E6-4), never holds an
/// unlimited account, and nothing recapitalizes it without an attributed act.
///
/// # Errors
///
/// [`MarketError::Economy`] carries the economy's own refusal, mint cap included.
pub fn fund_from_faucet(
    ctx: &mut Ctx<'_>,
    faucet: &mush_org::SystemPrincipalId,
    faucet_account: AccountId,
    credit: AccountId,
    amount: Amount,
    period_secs: i64,
    namespace: &str,
) -> Result<mush_economy::ObligationId, MarketError> {
    if amount.minor() <= 0 {
        return Err(MarketError::bad("A funding drip is a positive amount."));
    }
    let now = ctx.w.now();
    mush_economy::create_system_obligation(
        ctx,
        faucet,
        mush_economy::ObligationSpec {
            debit_account: faucet_account,
            credit_account: credit,
            amount,
            schedule: mush_economy::EconomicSchedule::new(
                EconomicClock::RealUtc,
                now.saturating_add(period_secs.max(60)),
                Some(period_secs.max(60)),
                1,
            ),
            grace: mush_economy::GracePolicy::default(),
            notify_on_success: false,
            cause: mush_economy::ObligationCause::Ruleset {
                namespace: SmolStr::new(namespace),
            },
        },
    )
    .map_err(MarketError::Economy)
}

// ---- B5: listings ----------------------------------------------------------

/// What `list_goods` was asked for.
#[derive(Clone, Debug)]
pub struct ListingSpec {
    /// Which vendor is selling.
    pub vendor: VendorId,
    /// What.
    pub catalog: CatalogRef,
    /// The asking price, per unit.
    pub price: Amount,
    /// How many.
    pub quantity: u64,
    /// Where the goods come from.
    pub source: Supply,
    /// The specific titled object, for a `Representation::Asset`.
    pub object: Option<ObjId>,
    /// Whether it is in the public projection at all.
    pub visibility: MarketVisibility,
    /// How long it stands, in seconds.
    pub ttl_secs: i64,
}

/// **Publish a listing** (B5).
///
/// A listing reserves nothing and escrows nothing: it is a published intention, and
/// its `quantity` decrements only at a purchase contract's **activation** (OPEN 4).
///
/// A listing whose catalog entry declares [`Representation::Lot`] is refused here
/// (OPEN 1): the lot is `mush-inventory`'s in E7 cut 2, with a different field list,
/// and E6c will not define a second one.
///
/// # Errors
///
/// [`MarketError`]; render it with [`refuse_market`].
pub fn list_goods(ctx: &mut Ctx<'_>, spec: ListingSpec) -> Result<ListingId, MarketError> {
    // Doc 18 cut IC8 (lead ruling L14): a standing bid is posted through its own entry,
    // never listed — refused FIRST, because a Wanted on a non-lot entry would otherwise
    // fall to the `(_, None, _)` arm below and publish as an ordinary counter listing.
    if spec.source.is_wanted() {
        return Err(MarketError::bad(
            "A standing bid is posted with post_wanted, never listed.",
        ));
    }
    let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(spec.vendor).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_VENDOR));
    };
    if !vendor.is_trading() {
        return Err(MarketError::bad(
            "That vendor has retired and is not trading.",
        ));
    }
    // DL-06 again (M4): a suspended, dissolving or closed organization publishes no new
    // offer. Its live contracts settle through E4 untouched.
    if !org_may_trade(ctx.w, vendor.org) {
        return Err(MarketError::bad(
            "That vendor's organization is not trading.",
        ));
    }
    let Some(entry) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&spec.catalog).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_ENTRY));
    };
    // **A listing may not name somebody else's market maker** (independent verify fold,
    // M8). The `stock` arm built `Supply::Market { source }` straight from the typed
    // number and nothing looked it up: not that it existed, not that it belonged to this
    // vendor's organization, not that it made a market in this venue. Every downstream
    // consumer then read the wrong maker — `price_for` took its spread, `quote` charged
    // `refuses` and the cash test against *its* wallet, `charge_budgets` spent and
    // returned *its* window — while the sale's money went to `vendor.account`: one shop
    // consuming another organization's declared risk budget, gated on a wallet it does
    // not own. A typo'd id was worse still, refusing every quote with `NO_SUCH_LISTING`
    // rather than naming the mistake.
    if let Supply::Market { source } = spec.source {
        let maker = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|s| s.source(source).cloned());
        match maker {
            None => return Err(MarketError::NoStanding(NO_SUCH_SOURCE)),
            Some(m) if m.principal != vendor.org => {
                return Err(MarketError::bad(
                    "That market maker belongs to another organization.",
                ));
            }
            Some(m) if m.market != vendor.market => {
                return Err(MarketError::bad(
                    "That market maker does not make a market in this vendor's venue.",
                ));
            }
            Some(_) => {}
        }
        // **A maker prices off a reference price, by construction** (independent verify
        // fold, M6). `CatalogEntry.baseline` is `Option`, and both loss sites resolved
        // `None` to zero — so `loss = (0 - price).max(0) * qty` was always zero and the
        // loss budget never tripped however far below the shelf price the spread took
        // the quote (down to -1000, i.e. a tenth). Only the volume budget, which shrinks
        // as the price does, was left, so doc 10 7A's "mediocre prices is a trading
        // policy, not a substitute for funding" was unenforced for any entry that omits
        // one.
        if entry.baseline.is_none_or(|b| b.minor() <= 0) {
            return Err(MarketError::bad(format!(
                "'{}' carries no reference price, and a market maker prices against one. Give the \
                 catalogue entry a baseline, or stock it from the vendor's own vault.",
                entry.name
            )));
        }
    }
    // ---- E7r2: P1 — the `Representation::Lot` refusal, LIFTED where it can be ----
    //
    // OPEN 1's reason is unchanged and is why the predicate survives: the lot is
    // `mush-inventory`'s single source of quantity truth (10b §3.2, Q2D), and defining
    // a second one here would invert the intended `mush-market -> mush-inventory`
    // edge. What changed is that the edge now EXISTS and the quantity is reached
    // through cut 2's own exported verbs on the delegated path, so a server that
    // composed `MarketModule::with_inventory` can publish one. A server that did not
    // refuses with the sentence E6c refused it with, byte for byte —
    // `ruleset-shadowrun/tests/e6c.rs`'s `"not stockable"` assertion is untouched.
    let lot_supply = crate::module::lot_supply(ctx.w);
    if !entry.representation.is_stockable() && lot_supply.is_none() {
        return Err(MarketError::bad(format!(
            "'{}' is a lot-represented good, and lots are not stockable on this server yet.",
            entry.name
        )));
    }
    // The two halves must agree in **both** directions: a lot-represented entry is
    // stocked from an inventory endpoint and nothing else is, so no other
    // representation can borrow the one `Supply` arm that reserves.
    //
    // Doc 18 cut IC7 (D-18-26, lead note 2b): a lot-represented entry stocked with no
    // `endpoint=` is a **Draw** listing — `Supply::Stock { vault }` on the vendor's own
    // storefront, nothing reserved, the listing's quantity the availability counter,
    // and the matter leg a transfer out of the storefront pool at activation.
    let lot_leg = match (entry.representation, spec.source.lot(), &spec.source) {
        (Representation::Lot, Some((endpoint, commodity, grade)), _) => {
            Some((endpoint, commodity.clone(), grade))
        }
        (Representation::Lot, None, Supply::Stock { vault }) => {
            draw_listing_precheck(ctx, &vendor, &spec, *vault)?;
            None
        }
        (Representation::Lot, None, _) => {
            return Err(MarketError::bad(
                "A lot-represented good is stocked out of an inventory endpoint \
                 (endpoint=<vault>) or drawn from the vendor's own storefront.",
            ));
        }
        (_, Some(_), _) => {
            return Err(MarketError::bad(
                "Only a lot-represented good is stocked out of an inventory endpoint.",
            ));
        }
        (_, None, _) => None,
    };
    if let Some((endpoint, commodity, _)) = &lot_leg {
        // **Custody, existence-hidingly.** `mush_inventory::may_take` is E3's own
        // vault-withdraw authority, asked through cut 2's exported seam: an actor who
        // may not take from the endpoint reads exactly what an actor naming an
        // endpoint that never existed reads. It is a PRE-CHECK for a good refusal —
        // the gate is the offer's own regenerated `Choice` list, which
        // `Engine::resolve_legal_action` rebuilds immediately before preparation.
        if !mush_inventory::may_take(ctx.w, ctx.actor, *endpoint) {
            return Err(MarketError::NoStanding(NO_SUCH_LISTING));
        }
        // **One live listing per pool** (the one-live-listing-per-lot index). Two
        // listings over one `(endpoint, commodity)` would each hold their own share of
        // the same pool — legal for K's contention set, and an oversell of the same
        // provenance root as far as a buyer can tell, because both would draw from the
        // one tranche and the second sale would refuse **after** its money leg was in
        // force. Refused here instead.
        if ctx
            .w
            .module_state::<MarketStore>()
            .is_some_and(|s| s.lot_listing_of(*endpoint, commodity).is_some())
        {
            return Err(MarketError::bad(
                "That endpoint already carries a live listing for that commodity. Withdraw \
                 it first (@market/vendor withdraw <listing>).",
            ));
        }
    }
    if spec.price.minor() <= 0 {
        return Err(MarketError::bad("A listing asks a positive price."));
    }
    if spec.quantity == 0 || spec.quantity > crate::record::MAX_QUANTITY {
        return Err(MarketError::bad(
            "That is not a quantity this market sells.",
        ));
    }
    // An `Asset` listing conveys exactly one **named, titled** object, because that
    // is what `Deliverable::TitleTransfer` moves. Every other representation conveys
    // no object at all.
    match (entry.representation, spec.object) {
        (Representation::Asset, None) => {
            return Err(MarketError::bad(
                "An asset listing names the object it conveys.",
            ));
        }
        (Representation::Asset, Some(o)) => {
            if !is_live_object(ctx.w, o) {
                return Err(MarketError::NoStanding(NO_SUCH_LISTING));
            }
            if spec.quantity != 1 {
                return Err(MarketError::bad("An asset listing sells exactly one."));
            }
            let titled = ctx
                .w
                .module_state::<EconomyStore>()
                .and_then(|s| s.snapshot().asset(o).cloned());
            match titled {
                Some(a)
                    if a.title_owner == PrincipalId::Organization(vendor.org)
                        && a.lifecycle == mush_economy::AssetLifecycle::Active
                        && !a.frozen => {}
                // **Stocking mints the title, stamped with its catalogue reference**
                // (B4a, B11). Without the stamp nothing the market sells could ever
                // be quoted by a fence — E6b's `price` reads the asset's `catalog`
                // and refuses an object that carries none — and every sale would
                // need a second staff act (`@asset/catalog`) to become sellable
                // again. `issue_title` refuses an issuer the composition did not
                // freeze, so a server that named none simply cannot stock assets.
                None => {
                    // **Minting title is not a side effect of publishing a listing**
                    // (independent verify fold, M10). This arm asked only
                    // `is_live_object` and `issue_title`'s own `approver_live` — the
                    // same test — so `@market/vendor stock <v>=<code> object=<a player
                    // character>` silently minted a vendor-owned title over that
                    // character, stamped with any loaded catalogue code, which is also
                    // what E6b's fence prices off. E6b's fence required the seller to be
                    // **holding** the object; this path had dropped that. Staff-only, so
                    // a footgun rather than a privilege breach — but a silent one, taken
                    // as a side effect of something else.
                    if !titleable(ctx, o, &vendor) {
                        return Err(MarketError::bad(
                            "A vendor stocks a thing it is holding, or one in its own vault. Title \
                             anything else deliberately, with @asset/title.",
                        ));
                    }
                    let Some(issuer) = crate::module::title_issuer(ctx) else {
                        return Err(MarketError::bad(
                            "This server registers no market title issuer, so it cannot stock \
                             titled goods.",
                        ));
                    };
                    mush_economy::issue_title(
                        ctx,
                        o,
                        PrincipalId::Organization(vendor.org),
                        None,
                        issuer,
                        Some(spec.catalog.clone()),
                    )
                    .map_err(MarketError::Economy)?;
                }
                Some(_) => {
                    return Err(MarketError::bad(
                        "That vendor does not hold a live title to that object.",
                    ));
                }
            }
        }
        (_, Some(_)) => {
            return Err(MarketError::bad("Only an asset listing names an object."));
        }
        (_, None) => {}
    }
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.listings_of_vendor(spec.vendor) >= MAX_LISTINGS_PER_VENDOR)
    {
        return Err(MarketError::bad(
            "That vendor already holds as many listings as the market allows.",
        ));
    }
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.listings.len() >= MAX_LISTINGS)
    {
        return Err(MarketError::bad("The market is full; try later."));
    }
    let mut ttl = spec.ttl_secs.clamp(60, MAX_LISTING_TTL_SECS);
    // **A lot listing cannot outlive its hold** (doc 18 cut IC0). The reservation
    // `lot::stage` places below lapses `RESERVATION_TTL_SECS` of **IC** time after it
    // is placed, while a listing's deadline is real time; before IC0 the command's
    // default was fourteen real days against seven IC days and nothing read the hold,
    // so a quote taken late took the money and the matter leg refused with it in
    // force. The listing's window is cut to the hold's, converted at the world's
    // current rate, so what the page advertises as for sale is what the hold still
    // backs; `trade::is_current` and `trade::quote` read the hold itself as well,
    // which is what holds when the rate moves after the listing is written.
    if lot_leg.is_some()
        && let Some(cap) = crate::lot::hold_ttl_real_secs(ctx.w)
    {
        ttl = ttl.min(cap).max(1);
    }
    let now = ctx.w.now();
    let Some(id) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_listing)
    else {
        return Err(MarketError::bad("The listing-id allocator is exhausted."));
    };
    put_listing(
        ctx,
        Listing {
            id,
            seller: PrincipalId::Organization(vendor.org),
            vendor: vendor.id,
            market: vendor.market.clone(),
            catalog: spec.catalog,
            definition_version: entry.definition_version,
            source: spec.source,
            object: spec.object,
            price: spec.price,
            quantity: spec.quantity,
            visibility: spec.visibility,
            expires: EconomicDeadline {
                clock: EconomicClock::RealUtc,
                at: now.saturating_add(ttl),
            },
            revision: 1,
        },
    );
    persist_meta(ctx);
    // ---- E7r2: P1 — the hold, staged AFTER the document that names it ----
    //
    // OPEN 2's ordering, and the reason for the sweep arm: a command cannot observe
    // its own authoritative submission, so the `ReservationId` is a pure derivation
    // (`lot::listing_reservation`) the listing document already implies, and the ONE
    // submit is staged last. A crash in between leaves a listing whose hold nobody
    // placed — which `boot::sweep`'s own arm withdraws and pages, per listing.
    if let Some((endpoint, commodity, grade)) = lot_leg {
        let staged = crate::lot::stage(
            ctx,
            &crate::lot::LotLeg {
                op: crate::lot::LotOp::Reserve,
                listing: id,
                endpoint,
                source: endpoint,
                commodity,
                quantity: spec.quantity,
                grade,
                contract: None,
            },
        );
        if !staged {
            // The composition wired a lot supply and the action is not registered:
            // that is a composition defect, not a player error. Take the document back
            // rather than leaving a listing nothing can move.
            del_listing(ctx, id);
            return Err(MarketError::bad(
                "This server cannot move lot stock right now, so nothing was listed.",
            ));
        }
    }
    Ok(id)
}

/// **A Draw listing's own checks, before any write** (doc 18 cut IC7, L11): it sells
/// out of the vendor's own storefront, which the actor may take from, under a catalogue
/// code that is a commodity id, on a pool no other live listing sits on, and the
/// storefront's **available** stock covers the listing (equality and drift are IC8's
/// readiness check). An unreadable storefront refuses rather than reading as empty.
fn draw_listing_precheck(
    ctx: &Ctx<'_>,
    vendor: &Vendor,
    spec: &ListingSpec,
    vault: mush_economy::VaultId,
) -> Result<(), MarketError> {
    if vault != vendor.vault {
        return Err(MarketError::bad(
            "A drawn listing sells out of the vendor's own storefront vault.",
        ));
    }
    let endpoint = mush_inventory::EndpointRef(vault);
    if !mush_inventory::may_take(ctx.w, ctx.actor, endpoint) {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    }
    let Ok(commodity) = mush_core::CanonicalId::new(spec.catalog.code.as_str()) else {
        return Err(MarketError::bad(
            "That catalogue code is not a commodity id.",
        ));
    };
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.lot_listing_of(endpoint, &commodity).is_some())
    {
        return Err(MarketError::bad(
            "That endpoint already carries a live listing for that commodity. Withdraw \
             it first (@market/vendor withdraw <listing>).",
        ));
    }
    match crate::lot::pool_available(ctx, endpoint, &commodity) {
        Ok(available) if available >= spec.quantity => Ok(()),
        Ok(_) => Err(MarketError::bad(
            "That storefront holds less than that, so nothing was listed.",
        )),
        Err(()) => Err(MarketError::bad(
            "That storefront's stock cannot be read right now, so nothing was listed.",
        )),
    }
}

/// Whether an object is one a vendor may have title **minted** over as a side effect of
/// stocking it (independent verify fold, M10).
///
/// E6b's fence rule, one layer up: it must be an ordinary `Thing` — never a character,
/// a room or an exit — and it must be where a shop's stock is, which is the vendor's own
/// vault backing object or the hands of the staffer publishing the listing. Anything
/// else is titled deliberately, with `@asset/title`, where the act is what it says it
/// is.
fn titleable(ctx: &Ctx<'_>, object: ObjId, vendor: &Vendor) -> bool {
    let Some(obj) = ctx.w.get(object) else {
        return false;
    };
    if !matches!(obj.kind, mush_core::ObjKind::Thing) {
        return false;
    }
    let backing = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().vault(vendor.vault).map(|v| v.backing.clone()))
        .and_then(|b| b.object());
    obj.location == Some(ctx.actor) || (backing.is_some() && obj.location == backing)
}

/// Take a listing down.
///
/// # Errors
///
/// [`MarketError::NoStanding`] when there is no such listing to the actor.
pub fn withdraw_listing(ctx: &mut Ctx<'_>, id: ListingId) -> Result<(), MarketError> {
    let Some(listing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(id).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    // ---- E7r2: P1 — a withdrawn lot listing gives its hold back ----
    //
    // The document delete and the release are one command's work and the release is
    // its ONE authoritative submission. The document goes first so a crash in between
    // leaves a hold with no listing, which the sweep's own arm pages about — the
    // opposite order would leave a listing with no hold, which is the shape a buyer
    // can pay for.
    let leg = listing
        .source
        .lot()
        .map(|(endpoint, commodity, grade)| crate::lot::LotLeg {
            op: crate::lot::LotOp::Release,
            listing: id,
            endpoint,
            source: endpoint,
            commodity: commodity.clone(),
            quantity: listing.quantity,
            grade,
            contract: None,
        });
    // A Draw (doc 18 cut IC7) and a standing bid (cut IC8) hold no reservation, so
    // deleting the document is the whole withdrawal and nothing is staged.
    del_listing(ctx, id);
    if let Some(leg) = leg {
        crate::lot::stage(ctx, &leg);
    }
    Ok(())
}

// ---- doc 18 cut IC8: standing bids -----------------------------------------

/// What [`post_wanted`] is asked to post (doc 18 cut IC8; §4.3 "Bids that take real
/// lots", posted as a document only — selling into it is cut IC10's).
#[derive(Clone, Debug)]
pub struct WantedSpec {
    /// The bidding vendor: its org is the listing's `seller`.
    pub vendor: VendorId,
    /// What it buys: a `Representation::Lot` entry whose code is a commodity id.
    pub catalog: CatalogRef,
    /// The bid per unit.
    pub price: Amount,
    /// The intake still wanted, `1..=MAX_QUANTITY` (the listing's `quantity`).
    pub remaining: u64,
    /// The vendor's own vault, where bought matter would be received.
    pub vault: VaultId,
    /// The lowest grade it takes.
    pub min_grade: u8,
    /// How long it stands, in real seconds (clamped to `60..=MAX_LISTING_TTL_SECS`).
    pub ttl_secs: i64,
}

/// **Post a standing bid** (doc 18 cut IC8, lead rulings L14 and L21): a
/// `Supply::Wanted` listing whose `seller` is the bidding org, `price` the bid per unit
/// and `quantity` the intake still wanted. Normal-lane: it reserves nothing, escrows
/// nothing and submits nothing (the bid's funds are read, not held — "not buying" is a
/// render, never a refusal here).
///
/// Refused, before any write: a vendor that is not trading or whose org may not trade;
/// an entry that is not `Representation::Lot` or whose code is not a commodity id; a
/// vault that is not the vendor's own; a non-positive price, an out-of-range intake or
/// grade floor; a second live bid of this vendor for this commodity; the listing caps.
///
/// # Errors
///
/// [`MarketError`]; render it with [`refuse_market`].
pub fn post_wanted(ctx: &mut Ctx<'_>, spec: WantedSpec) -> Result<ListingId, MarketError> {
    let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(spec.vendor).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_VENDOR));
    };
    if !vendor.is_trading() || !org_may_trade(ctx.w, vendor.org) {
        return Err(MarketError::bad(
            "That vendor is not trading, so it posts no bid.",
        ));
    }
    let Some(entry) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&spec.catalog).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_ENTRY));
    };
    if entry.representation != Representation::Lot {
        return Err(MarketError::bad(
            "A standing bid buys a lot-represented good.",
        ));
    }
    let Ok(commodity) = mush_core::CanonicalId::new(spec.catalog.code.as_str()) else {
        return Err(MarketError::bad(
            "That catalogue code is not a commodity id.",
        ));
    };
    if spec.vault != vendor.vault {
        return Err(MarketError::bad(
            "A standing bid receives into the vendor's own storefront vault.",
        ));
    }
    if spec.price.minor() <= 0 {
        return Err(MarketError::bad("A standing bid offers a positive price."));
    }
    if spec.remaining == 0 || spec.remaining > crate::record::MAX_QUANTITY {
        return Err(MarketError::bad("That is not a quantity this market buys."));
    }
    if spec.min_grade > mush_inventory::MAX_GRADE {
        return Err(MarketError::bad("That grade floor is out of range."));
    }
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.wanted_of(vendor.id, &commodity).is_some())
    {
        return Err(MarketError::bad(
            "That vendor already posts a standing bid for that good.",
        ));
    }
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.listings_of_vendor(vendor.id) >= MAX_LISTINGS_PER_VENDOR)
    {
        return Err(MarketError::bad(
            "That vendor already holds as many listings as the market allows.",
        ));
    }
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.listings.len() >= MAX_LISTINGS)
    {
        return Err(MarketError::bad("The market is full; try later."));
    }
    let ttl = spec.ttl_secs.clamp(60, MAX_LISTING_TTL_SECS);
    let now = ctx.w.now();
    let Some(id) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_listing)
    else {
        return Err(MarketError::bad("The listing-id allocator is exhausted."));
    };
    put_listing(
        ctx,
        Listing {
            id,
            seller: PrincipalId::Organization(vendor.org),
            vendor: vendor.id,
            market: vendor.market.clone(),
            catalog: spec.catalog,
            definition_version: entry.definition_version,
            source: Supply::Wanted {
                vault: spec.vault,
                commodity: crate::record::LotCommodity(commodity),
                min_grade: spec.min_grade,
            },
            object: None,
            price: spec.price,
            quantity: spec.remaining,
            visibility: MarketVisibility::Public,
            expires: EconomicDeadline {
                clock: EconomicClock::RealUtc,
                at: now.saturating_add(ttl),
            },
            revision: 1,
        },
    );
    persist_meta(ctx);
    Ok(id)
}

/// **Revise a standing bid in place** (doc 18 cut IC8): a new price and remaining
/// intake on a live `Supply::Wanted` listing, its revision bumped. Its deadline is
/// unchanged.
///
/// # Errors
///
/// [`MarketError::NoStanding`] when there is no such bid; [`MarketError::Bad`] for a
/// non-positive price or an out-of-range intake.
pub fn revise_wanted(
    ctx: &mut Ctx<'_>,
    id: ListingId,
    price: Amount,
    remaining: u64,
) -> Result<(), MarketError> {
    let Some(listing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(id).cloned())
        .filter(|l| l.source.is_wanted())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    if price.minor() <= 0 {
        return Err(MarketError::bad("A standing bid offers a positive price."));
    }
    if remaining == 0 || remaining > crate::record::MAX_QUANTITY {
        return Err(MarketError::bad("That is not a quantity this market buys."));
    }
    put_listing(
        ctx,
        Listing {
            price,
            quantity: remaining,
            revision: listing.revision.saturating_add(1),
            ..listing
        },
    );
    Ok(())
}

// ---- the per-viewer half: who may see a listing -----------------------------

/// Whether `viewer` may see this listing **at all**.
///
/// The per-viewer half of §5.3, and the only viewer-sensitive read in the cut. A
/// listing fails it three ways, and all three answer with the same
/// [`NO_SUCH_LISTING`] text: it is `Hidden` and the viewer is neither its seller nor
/// staff; its selling organization is hidden from the viewer; or it does not exist.
#[must_use]
pub fn may_view_listing(w: &World, viewer: ObjId, l: &Listing) -> bool {
    if is_staff(w, viewer) {
        return true;
    }
    if mush_economy::acts_for(w, viewer, &l.seller) {
        return true;
    }
    if !matches!(l.visibility, MarketVisibility::Public) {
        return false;
    }
    // A hidden vendor's listings are **absent**, never refused (G3): the economy owns
    // the render, the market owns the omission.
    match &l.seller {
        PrincipalId::Organization(org) => list_org(w, viewer, *org),
        PrincipalId::Object(_) => true,
        PrincipalId::System(_) => false,
    }
}

/// Whether `viewer` may see this **vendor** at all — the same rule as a listing's,
/// asked of the shop rather than the shelf. A hidden vendor is absent from
/// `+market/vendors` for a non-member rather than refused (G3).
#[must_use]
pub fn may_list_vendor(w: &World, viewer: ObjId, v: &Vendor) -> bool {
    list_org(w, viewer, v.org)
}

/// `mush_economy::may_list_org` against a `&World` — the same pair, asked without a
/// `&mut Ctx`, so a read path never needs one.
#[must_use]
fn list_org(w: &World, viewer: ObjId, org: OrgId) -> bool {
    if !is_hidden_org(w, org) {
        return true;
    }
    is_staff(w, viewer) || mush_economy::acts_for(w, viewer, &PrincipalId::Organization(org))
}

/// One page of the market, newest first, from `after` (exclusive).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarketPage {
    /// The listings this page shows.
    pub ids: Vec<ListingId>,
    /// Where the next page starts, when the walk stopped before the end.
    pub next: Option<ListingId>,
    /// How many records the walk **visited** — what [`MARKET_PAGE_VISIT`] bounds.
    pub visited: usize,
}

/// The listings `viewer` may see, newest first (§5.3's per-viewer half).
///
/// **Bounded on records VISITED, not records matched** (D-LBK-5; E6b verify fold D3's
/// "filter, then page"). An unauthorized or unknown filter returns an **empty list**,
/// never a refusal (08a decision 13), and the page never exceeds
/// [`MARKET_PAGE_MAX`] rows (08i §9 answer 11).
#[must_use]
pub fn visible(
    w: &World,
    viewer: ObjId,
    filter: Option<&MarketFilter>,
    page: usize,
    after: Option<ListingId>,
) -> MarketPage {
    let Some(store) = w.module_state::<MarketStore>() else {
        return MarketPage::default();
    };
    let now = w.now();
    let page = page.clamp(1, MARKET_PAGE_MAX);
    let mut out = MarketPage::default();
    let mut last: Option<ListingId> = None;
    let walk: Box<dyn Iterator<Item = &Listing>> = match after {
        Some(cursor) => Box::new(
            store
                .live
                .range(..cursor)
                .rev()
                .filter_map(|id| store.listing(*id)),
        ),
        None => Box::new(store.live_ids_desc().filter_map(|id| store.listing(id))),
    };
    for l in walk {
        if out.visited >= MARKET_PAGE_VISIT || out.ids.len() >= page {
            out.next = last;
            return out;
        }
        out.visited = out.visited.saturating_add(1);
        last = Some(l.id);
        // **The page and the projection agree about what is gone** (independent verify
        // fold, R3 and R6). `MarketStore::reindex` builds `live` from `quantity > 0`
        // alone, so a lapsed listing stayed in it and the page rendered it as buyable
        // while `quote` refused it; and a pack reload that dropped an entry left
        // listings nothing can define, quote or deliver. The sweep deletes both, and
        // this is what makes the render honest between two fires.
        let closed = crate::projection::lapsed(l.expires.at, l.expires.clock, now)
            || store.entry_of(&l.catalog).is_none();
        let matches = !closed && filter.is_none_or(|f| f.matches(store, l));
        if matches && may_view_listing(w, viewer, l) {
            out.ids.push(l.id);
        }
    }
    out
}

/// What `+market/list` was asked to narrow to.
///
/// An **unknown** filter word produces [`MarketFilter::Nothing`], which matches
/// nothing and therefore renders an empty list — never a refusal, and never a
/// different shape from "there is nothing here" (08a decision 13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarketFilter {
    /// Only this legality class.
    Legal(Legality),
    /// Only entries carrying this tag.
    Tag(SmolStr),
    /// Only this market.
    Market(MarketId),
    /// A filter the actor may not use, or one this server does not know.
    Nothing,
}

impl MarketFilter {
    /// Parse one `key=value` filter word.
    #[must_use]
    pub fn parse(s: &str) -> MarketFilter {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("legal=") {
            return Legality::parse(rest).map_or(MarketFilter::Nothing, MarketFilter::Legal);
        }
        if let Some(rest) = s.strip_prefix("tag=") {
            let rest = rest.trim();
            if rest.is_empty() || rest.len() > crate::record::MAX_TAG_BYTES {
                return MarketFilter::Nothing;
            }
            return MarketFilter::Tag(SmolStr::new(rest));
        }
        if let Some(rest) = s.strip_prefix("market=") {
            return MarketId::new(rest).map_or(MarketFilter::Nothing, MarketFilter::Market);
        }
        MarketFilter::Nothing
    }

    fn matches(&self, store: &MarketStore, l: &Listing) -> bool {
        match self {
            MarketFilter::Nothing => false,
            MarketFilter::Market(m) => l.market == *m,
            MarketFilter::Legal(want) => store
                .entry_of(&l.catalog)
                .is_some_and(|e| e.legality == *want),
            MarketFilter::Tag(t) => store
                .entry_of(&l.catalog)
                .is_some_and(|e| e.tags.contains(t)),
        }
    }
}

// ---- B8: NPC market makers --------------------------------------------------

/// **Register an NPC market maker** with explicit, exhaustible budgets (B8, doc 10
/// 7A, D-E6-15).
///
/// # Errors
///
/// [`MarketError`]; render it with [`refuse_market`].
#[allow(clippy::too_many_arguments)]
pub fn create_source(
    ctx: &mut Ctx<'_>,
    instrument: &InstrumentId,
    principal: OrgId,
    inventory: VaultId,
    market: MarketId,
    spread: i64,
    volume_budget: Amount,
    loss_budget: Amount,
    window: i64,
) -> Result<MarketSourceId, MarketError> {
    // D-E6-4, structurally: a maker is an `Organization`, never a `System` principal,
    // and it holds an ordinary account nobody can overdraw.
    let who = PrincipalId::Organization(principal);
    let cash =
        principal_account(ctx, instrument, &who).ok_or(MarketError::NoStanding(NO_SUCH_SOURCE))?;
    let ok = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().vault(inventory).cloned())
        .is_some_and(|v| v.owner == who && v.status == VaultStatus::Active);
    if !ok {
        return Err(MarketError::NoStanding(NO_SUCH_SOURCE));
    }
    if !(-1_000..=1_000).contains(&spread) {
        return Err(MarketError::bad(
            "A maker's spread is a permille of the catalogue baseline, -1000..=1000.",
        ));
    }
    if volume_budget.minor() < 0 || loss_budget.minor() < 0 {
        return Err(MarketError::bad("A maker's budgets are never negative."));
    }
    if !(60..=365 * 86_400).contains(&window) {
        return Err(MarketError::bad(
            "A maker's budget window is 60 seconds to a year.",
        ));
    }
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.sources().count() >= MAX_SOURCES)
    {
        return Err(MarketError::bad(
            "This server already registers as many market makers as it allows.",
        ));
    }
    let Some(id) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_source)
    else {
        return Err(MarketError::bad("The maker-id allocator is exhausted."));
    };
    let now = ctx.w.now();
    put_source(
        ctx,
        MarketSource {
            id,
            principal,
            market,
            cash,
            inventory,
            spread,
            volume_budget,
            loss_budget,
            window,
            volume_used: Amount::ZERO,
            loss_used: Amount::ZERO,
            window_started: now,
            halted: false,
            revision: 1,
        },
    );
    persist_meta(ctx);
    Ok(id)
}

/// Re-budget a maker.
///
/// # Errors
///
/// [`MarketError::NoStanding`] for an unknown maker, [`MarketError::Bad`] for a
/// budget out of range.
pub fn budget_source(
    ctx: &mut Ctx<'_>,
    id: MarketSourceId,
    volume_budget: Option<Amount>,
    loss_budget: Option<Amount>,
    spread: Option<i64>,
) -> Result<(), MarketError> {
    let Some(s) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|st| st.source(id).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_SOURCE));
    };
    if volume_budget.is_some_and(|a| a.minor() < 0) || loss_budget.is_some_and(|a| a.minor() < 0) {
        return Err(MarketError::bad("A maker's budgets are never negative."));
    }
    if spread.is_some_and(|p| !(-1_000..=1_000).contains(&p)) {
        return Err(MarketError::bad("A maker's spread is -1000..=1000."));
    }
    put_source(
        ctx,
        MarketSource {
            volume_budget: volume_budget.unwrap_or(s.volume_budget),
            loss_budget: loss_budget.unwrap_or(s.loss_budget),
            spread: spread.unwrap_or(s.spread),
            revision: s.revision.saturating_add(1),
            ..s
        },
    );
    Ok(())
}

/// Halt (or resume) a maker. **A halted maker's outstanding quotes expire and settle
/// nothing**: the quote's own `is_current` asks the maker, so halting is what makes
/// every one of them refuse deterministically at accept.
///
/// # Errors
///
/// [`MarketError::NoStanding`] for an unknown maker.
pub fn halt_source(ctx: &mut Ctx<'_>, id: MarketSourceId, halted: bool) -> Result<(), MarketError> {
    let Some(s) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|st| st.source(id).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_SOURCE));
    };
    put_source(
        ctx,
        MarketSource {
            halted,
            revision: s.revision.saturating_add(1),
            ..s
        },
    );
    Ok(())
}
