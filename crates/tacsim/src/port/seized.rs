//! **Seized goods go to the enforcing authority** (doc 50 unit LW.5; Q-50-44 as corrected, doc
//! 50a C-8, A-5).
//!
//! A seizure is a transfer, never a sink: the goods a customs ruling takes from a hold are
//! credited to the enforcing authority's vault in the ruling's own transaction
//! (`InventoryFamilies::seize` with a destination; a hold's exact lots are relocated there), so
//! the matter census's `seized` term stays at zero and `balance` holds. The authority then
//! offers them at its port, a drawn ask from that vault ([`offer`]).
//!
//! - **The authority** ([`seizing_authority`]) is the enforcing authority's payee. Today, before
//!   the Kirkwood flip, it is the Freeport Registry, which issues the licence the picket checks,
//!   at its port. KC.2 points this one lookup at the Republic customs office once control of the
//!   approach flips (Q-50-14); nothing else names the payee.
//! - **Its vault** ([`ensure_vault`]): an authority that already sells at its port (a vendor, as
//!   Stickney Yards is) takes the goods into its storefront. One that does not, as the Registry
//!   does not at Ceres, is given a vault of its own and registered as the port's vendor on first
//!   use, at runtime, so no lane content changes.
//! - **The price** ([`SEIZED_PRICE_PERMILLE`]): provisional, three quarters of the good's
//!   reference ("seized ore for sale at a discount is something a trader can find").

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{AccountId, CatalogRef, CustodyRef, EconomyStore, Vault, VaultId, VaultStatus};

use mush_market::{MarketId, MarketStore, MarketVisibility, Supply};
use mush_org::{OrgId, PrincipalId};
use smol_str::SmolStr;

/// **What a seized good is offered at**, per mille of its reference price (provisional).
pub const SEIZED_PRICE_PERMILLE: u64 = 750;

/// **Who takes and sells seized goods**: an organization and the port it sells at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeizingAuthority {
    /// The organization.
    pub org: OrgId,
    /// Its port's lane entry.
    pub entry: CanonicalId,
    /// Its port's venue.
    pub venue: MarketId,
    /// Its account (the fine's payee), when it has one.
    pub account: Option<AccountId>,
}

/// **The enforcing authority for a seizure at the picket** (Q-50-44, Q-50-14): before the
/// Kirkwood flip, the Freeport Registry at the port that issues the licence. KC.2 re-points this
/// lookup at the Republic customs office after the flip. `None` before the Registry is made.
#[must_use]
pub fn seizing_authority(w: &World) -> Option<SeizingAuthority> {
    let (org, _, account) = crate::space::picket::resolve_registry_principal_and_account(w)?;
    let (def, map, _) = crate::port::licence::licence_port(w)?;
    Some(SeizingAuthority {
        org,
        entry: map.entry.clone(),
        venue: MarketId::new(def.venue.as_str())?,
        account: Some(account),
    })
}

/// **The authority's vault**, made (with its market vendor) on first use. An authority that is a
/// vendor already takes seized goods into its storefront.
///
/// # Errors
///
/// The economy or the market is not running, or the vendor will not register.
pub fn ensure_vault(ctx: &mut Ctx<'_>, authority: &SeizingAuthority) -> Result<VaultId, String> {
    if let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor_of_org(authority.org))
    {
        return Ok(vendor.vault);
    }
    let root = ctx
        .w
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().org(authority.org).map(|o| o.root_scope))
        .ok_or("the authority's organization is not live")?;
    let name =
        crate::surface::retrieval::principal_name(ctx.w, &PrincipalId::Organization(authority.org));
    let vault = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .ok_or("the economy is not running")?
        .alloc_vault()
        .ok_or("no vault can be opened right now")?;
    let store_name = format!("{name} seized goods");
    let container = ctx
        .w
        .create(
            mush_core::ObjKind::Thing,
            &store_name,
            mush_core::GOD,
            Some(mush_core::LIMBO),
        )
        .map_err(|e| format!("the seized-goods store cannot be made: {e}"))?;
    mush_economy::create_vault(
        ctx,
        Vault {
            id: vault,
            owner: PrincipalId::Organization(authority.org),
            scope: Some(root),
            name: SmolStr::new(&store_name),
            backing: CustodyRef::Container(container),
            status: VaultStatus::Active,
            revision: 1,
        },
    );
    let instrument = mush_economy::InstrumentId::new(crate::CREDIT).ok_or("no credit")?;
    mush_market::register_vendor(
        ctx,
        &instrument,
        authority.org,
        vault,
        authority.venue.clone(),
        SmolStr::new(name),
        0,
    )
    .map_err(|e| format!("the authority's vendor will not register: {e:?}"))?;
    // The port's stock clerk lists and draws from the vault: its direct `vault.withdraw` grant
    // on the authority's root scope, as the content applier gives every vendor (lead ruling L25).
    let clerk = port_clerk(ctx.w, &authority.entry).ok_or("the authority's port has no clerk")?;
    let grant = crate::port::apply::alloc_grant_id(ctx)?;
    let capability = mush_org::CapabilityId::new(crate::port::vendor::CLERK_CAPABILITY)
        .ok_or("the clerk's capability is not an id")?;
    let plan = mush_org::OrgMutationPlan::of(vec![mush_org::OrgMutationOp::PutGrant {
        grant: mush_org::CapabilityGrant {
            id: grant,
            org: authority.org,
            grantee: clerk,
            capability,
            scope: mush_org::ScopeSelector::Exact(root),
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
        mush_org::OrgAction {
            action: SmolStr::new_static("grant.create"),
            org: Some(authority.org),
            detail: serde_json::json!({
                "seized": authority.entry.as_str(),
                "clerk": clerk.0,
            }),
        },
    )
    .map_err(|e| format!("the clerk's grant on the seized goods: {e}"))?;
    Ok(vault)
}

/// **Whether `org` sells seized goods at `venue`** (the venue gate's port seller): it is the
/// seizing authority there and the market's vendor in that venue.
#[must_use]
pub fn sells_at(w: &World, venue: &MarketId, org: OrgId) -> bool {
    seizing_authority(w).is_some_and(|a| a.org == org && &a.venue == venue)
        && w.module_state::<MarketStore>()
            .and_then(|s| s.vendor_of_org(org))
            .is_some_and(|v| &v.market == venue)
}

/// The stock clerk of the port at `entry`.
fn port_clerk(w: &World, entry: &CanonicalId) -> Option<ObjId> {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(id, _, _)| id == entry)
        .and_then(|(_, _, map)| map?.clerk.as_ref().map(|c| ObjId(c.thing)))
}

/// **The price a seized `good` is offered at**, minor units: [`SEIZED_PRICE_PERMILLE`] of its
/// live reference (the port's reference reading, as a haul's declared value reads it). `None`
/// for a good with no reference.
#[must_use]
pub fn price_minor(w: &World, good: &CanonicalId) -> Option<i64> {
    let reference = crate::definitions::mass::good_reference_minor(w, good.as_str());
    let price = reference.saturating_mul(SEIZED_PRICE_PERMILLE) / 1_000;
    (price > 0).then(|| i64::try_from(price).ok()).flatten()
}

/// **Offer what `vault` holds of `good` at the authority's port** (Q-50-44): its standing ask
/// raised by `units`, or, with none, one ask of the whole pool, posted as the port's stock clerk.
/// A refusal is logged and the goods stay in the vault for the next seizure's offer.
pub fn offer(
    ctx: &mut Ctx<'_>,
    authority: &SeizingAuthority,
    vault: VaultId,
    good: &CanonicalId,
    units: u64,
) {
    if units == 0 {
        return;
    }
    let endpoint = mush_inventory::record::EndpointRef(vault);
    let Some(store) = ctx.w.module_state::<MarketStore>() else {
        return;
    };
    let Some(vendor) = store.vendor_of_org(authority.org).map(|v| v.id) else {
        tracing::warn!(
            vault = vault.0,
            "tacsim: seized goods with no vendor to offer them"
        );
        return;
    };
    let standing = store.lot_listing_of(endpoint, good).filter(|id| {
        store.listing(*id).is_some_and(|l| {
            matches!(l.source, Supply::Stock { .. }) && !mush_market::listing_lapsed(l, ctx.w.now())
        })
    });
    let Some(clerk) = port_clerk(ctx.w, &authority.entry) else {
        tracing::warn!(entry = %authority.entry, "tacsim: the authority's port has no clerk to offer seized goods");
        return;
    };
    let mut fire = crate::port_tick::Fire::new(std::collections::BTreeMap::new());
    let outcome = match standing {
        Some(listing) => fire
            .as_principal(ctx, clerk, clerk, |ctx| {
                mush_market::restock_ask(
                    ctx,
                    mush_market::RestockSpec {
                        listing,
                        good: good.clone(),
                        units,
                    },
                )
                .map(|_| ())
            })
            .map_err(|e| format!("{e:?}")),
        None => {
            let pooled = crate::port_tick::read_pool(ctx, endpoint, good).unwrap_or(0);
            let Some(price) = price_minor(ctx.w, good) else {
                tracing::warn!(good = %good, "tacsim: a seized good has no reference price; it is not offered");
                return;
            };
            let spec = mush_market::ListingSpec {
                vendor,
                catalog: CatalogRef {
                    ruleset: SmolStr::new_static(crate::NS),
                    code: SmolStr::new(good.as_str()),
                },
                price: mush_economy::Amount::from_minor(price),
                quantity: pooled.min(mush_market::MAX_QUANTITY),
                source: Supply::Stock { vault },
                object: None,
                visibility: MarketVisibility::Public,
                ttl_secs: mush_market::MAX_LISTING_TTL_SECS,
                private_to: None,
            };
            fire.as_principal(ctx, clerk, clerk, |ctx| {
                mush_market::list_goods(ctx, spec).map(|_| ())
            })
            .map_err(|e| format!("{e:?}"))
        }
    };
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(vault = vault.0, good = %good, ?error, "tacsim: seized goods could not be offered");
        }
        Err(error) => {
            tracing::warn!(vault = vault.0, good = %good, %error, "tacsim: seized goods could not be offered now");
        }
    }
}
