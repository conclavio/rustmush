//! **A recovery house's resale** (doc 41 cut RS.6c-2; §4.4 "On its own account", Q-41-16; the
//! owner's default of 2026-11-04: the sale is the buyout's contract shape).
//!
//! A hull the house kept ([`crate::port::house_sweep`]), restored by its arm
//! ([`crate::port::house_arm`]) and titled to the house, stands berthed in its home workshop and
//! is **for sale** at her design's reference price: `+salvage/forsale` lists the houses' stock,
//! and `+salvage/buy <hull> go` files the sale. The buyer offers one contract (D-18-21's shape,
//! [`crate::port::buyout`]'s): the price escrowed from the buyer's account, paid to the house's
//! when the house still holds her title, her title passing to the buyer; the house accepts
//! through the port's party caps, and the title passes at the next settlement. **Nothing is
//! stored**: the listing is read from where she stands and who holds her title, and a hull with
//! a live sale contract is not offered twice. A live sale is known by its shape, never its name
//! ([`pending`]; doc 48 §4.3).

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{
    AcceptOutcome, AccountId, Amount, ContractSpec, ContractState, Deliverable, EconomicContract,
    EconomyStore, PartyCap, PartyCapSource,
};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::port::coop::money_minor;
use crate::surface::retrieval::{acts_for, port_name, principal_name, split_go};

/// **The sale's window**, real seconds: the house's acceptance is immediate; an hour bounds a
/// sale whose title check fails (it is refunded at expiry), as the buyout's does.
pub const SALE_WINDOW_SECS: i64 = 3_600;

/// The listing's head.
pub const FORSALE_HEAD: &str = "Hulls for sale by the recovery houses:";
/// One listing: `{hull}`, `{name}`, `{house}`, `{port}`, `{price}`.
pub const FORSALE_LINE: &str = "  {hull}  {name}, restored by {house} at {port}: {price}.";
/// Nothing for sale.
pub const NONE_FOR_SALE: &str = "No recovery house has a hull for sale.";
/// The buy verb's usage.
pub const BUY_USAGE: &str = "Usage: +salvage/buy <hull> [go]";
/// Not on the list: `{hull}`.
pub const NOT_FOR_SALE: &str = "{hull} is not for sale; +salvage/forsale lists what is.";
/// Already being sold.
pub const SALE_PENDING: &str =
    "The {name} is already being sold; her title passes at the next settlement.";
/// The house selling to itself.
pub const OWN_SALE: &str = "You act for {house}; it cannot sell her to you.";
/// The preview: `{name}`, `{house}`, `{port}`, `{price}`.
pub const BUY_PREVIEW: &str = "{house} sells the {name}, berthed in its workshop at {port}, for {price}. Her title passes to you at the next settlement; she stays berthed until you stow her. Add go to buy her.";
/// The funds: `{price}`, `{available}`.
pub const BUY_FUNDS: &str = "The {name} costs {price}; you have {available} available.";
/// The sale could not be written.
pub const SALE_FAILED: &str =
    "The Registry could not write the sale just now; nothing changed, and staff have been told.";
/// Filed: `{name}`, `{house}`, `{price}`.
pub const SALE_FILED: &str = "The Registry writes the sale of the {name}: {price} is held against it and {house} countersigns. Her title passes to you at the next settlement.";

/// **A hull for sale.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// The hull.
    pub hull: ObjId,
    /// The house's home port.
    pub port: CanonicalId,
    /// The house (its operator organization).
    pub house: PrincipalId,
    /// The house's account, which the price is paid into.
    pub account: AccountId,
    /// Her price, minor units: her design's reference price.
    pub price_minor: u64,
}

/// The sale contract's name for `hull`: attribution only, never what marks a sale live.
fn sale_name(hull: ObjId) -> SmolStr {
    SmolStr::new(format!("Recovery house resale, #{}", hull.0))
}

/// **Whether `c` is a live sale of `hull` by `house`**, by its shape (doc 48 §4.3, from the
/// second review, doc 48b): past its draft and not yet terminal, `house` a party that has
/// accepted it as it stands, and a deliverable passing `hull`'s title from `house`. The name is
/// not consulted: a contract's name is a free label anyone may draft, so matching on it let a
/// player's own `@contract/draft` hide a hull from `+salvage/forsale` and refuse its
/// `+salvage/buy`. Only the port's party caps accept for the house, in `+salvage/buy`, so an
/// offer a player makes the house cannot pass for a sale either.
#[must_use]
pub fn is_sale_of(c: &EconomicContract, hull: ObjId, house: &PrincipalId) -> bool {
    !matches!(c.state, ContractState::Draft)
        && !c.state.is_terminal()
        && c.consent.parties.contains(house)
        && c.accepted_by(house)
        && c.consent.terms.deliverables.iter().any(|d| {
            matches!(d, Deliverable::TitleTransfer { object, from, .. }
                if *object == hull && from == house)
        })
}

/// **Whether a sale of `hull` by `house` is live** ([`is_sale_of`]).
#[must_use]
pub fn pending(w: &World, hull: ObjId, house: &PrincipalId) -> bool {
    w.module_state::<EconomyStore>().is_some_and(|economy| {
        economy
            .snapshot()
            .contracts()
            .any(|c| is_sale_of(c, hull, house))
    })
}

/// **The hulls the houses have for sale**: each berthed in a house's home workshop, titled to
/// its operator, not scrapped and not under an open order, at her design's reference price.
#[must_use]
pub fn for_sale(w: &World) -> Vec<Listing> {
    let mut listings = Vec::new();
    for (entry, def, map) in crate::port::market::ports(w) {
        let Some(map) = map else { continue };
        if map.house.is_none() {
            continue;
        }
        let Some(org) = def
            .recovery
            .as_ref()
            .and_then(|recovery| map.orgs.get(&recovery.operator))
        else {
            continue;
        };
        let Some(account) = org.account else { continue };
        let house = PrincipalId::Organization(mush_org::OrgId(org.org));
        let Some(room) = crate::port::workshop::room(w, &entry, def).ok().flatten() else {
            continue;
        };
        for hull in w.contents_of(room).to_vec() {
            if crate::port::house_sweep::title_of(w, hull).as_ref() != Some(&house)
                || crate::port::bay_work::scrapped(w, hull)
                || crate::port::bay_work::under_work(w, hull)
            {
                continue;
            }
            let Some((design, budget)) = crate::definitions::mass::hull_budget(w, hull) else {
                continue;
            };
            listings.push(Listing {
                hull,
                port: entry.clone(),
                house: house.clone(),
                account: AccountId(account),
                price_minor: crate::definitions::mass::reference_price_minor(w, &design, &budget),
            });
        }
    }
    listings
}

/// **The sale contract**: the market's one asset sale (doc 48 cut EX.2,
/// [`mush_market::asset_sale_spec`]; the buyout's shape), the house selling. Pure, so the
/// shape is tested without a world.
#[must_use]
pub(crate) fn sale_spec(
    buyer_account: AccountId,
    listing: &Listing,
    buyer: ObjId,
    now: i64,
) -> ContractSpec {
    mush_market::asset_sale_spec(&mush_market::AssetSaleTerms {
        name: sale_name(listing.hull),
        outcome: SmolStr::new_static("sold"),
        object: listing.hull,
        seller: listing.house.clone(),
        seller_account: listing.account,
        buyer: PrincipalId::Object(buyer),
        buyer_account,
        price: Amount::from_minor(i64::try_from(listing.price_minor).unwrap_or(i64::MAX)),
        deadline: now.saturating_add(SALE_WINDOW_SECS),
        others: Vec::new(),
        facts: Vec::new(),
    })
}

/// A name for `who`.
fn named(w: &World, who: ObjId) -> String {
    principal_name(w, &PrincipalId::Object(who))
}

fn say(ctx: &mut Ctx<'_>, text: String) {
    ctx.out.sys_to_actor(ctx.actor, "tacsim.salvage", text);
}

/// **`+salvage/forsale`**: the houses' hulls for sale.
///
/// # Errors
///
/// None today; the signature is the switch table's.
pub(crate) fn forsale_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = for_sale(ctx.w)
        .into_iter()
        .filter(|l| !pending(ctx.w, l.hull, &l.house))
        .map(|l| {
            FORSALE_LINE
                .replace("{hull}", &format!("#{}", l.hull.0))
                .replace("{name}", &named(ctx.w, l.hull))
                .replace("{house}", &principal_name(ctx.w, &l.house))
                .replace("{port}", &port_name(ctx.w, &l.port))
                .replace(
                    "{price}",
                    &money_minor(ctx, i64::try_from(l.price_minor).unwrap_or(i64::MAX)),
                )
        })
        .collect();
    let text = if lines.is_empty() {
        NONE_FOR_SALE.to_owned()
    } else {
        format!("{FORSALE_HEAD}\n{}", lines.join("\n"))
    };
    say(ctx, text);
    Ok(())
}

/// **`+salvage/buy <hull> [go]`**: preview the sale, or with `go` file it — offered and escrowed
/// by the buyer, accepted by the house.
///
/// # Errors
///
/// The refusal.
pub(crate) fn buy_cmd(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let (word, go) = split_go(raw);
    let word = word.trim();
    if word.is_empty() {
        return Err(CmdError::msg(BUY_USAGE));
    }
    let not_for_sale = || CmdError::msg(NOT_FOR_SALE.replace("{hull}", word));
    let hull = word
        .trim_start_matches('#')
        .parse::<u32>()
        .ok()
        .map(ObjId)
        .ok_or_else(not_for_sale)?;
    let listing = for_sale(ctx.w)
        .into_iter()
        .find(|l| l.hull == hull)
        .ok_or_else(not_for_sale)?;
    let name = named(ctx.w, hull);
    let house = principal_name(ctx.w, &listing.house);
    if pending(ctx.w, hull, &listing.house) {
        return Err(CmdError::msg(SALE_PENDING.replace("{name}", &name)));
    }
    if acts_for(ctx.w, ctx.actor, &listing.house) {
        return Err(CmdError::msg(OWN_SALE.replace("{house}", &house)));
    }
    let price_minor = i64::try_from(listing.price_minor).unwrap_or(i64::MAX);
    let price = money_minor(ctx, price_minor);
    if !go {
        let line = BUY_PREVIEW
            .replace("{house}", &house)
            .replace("{name}", &name)
            .replace("{port}", &port_name(ctx.w, &listing.port))
            .replace("{price}", &price);
        say(ctx, line);
        return Ok(());
    }
    let account = crate::port::credit::own_account(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You hold no credit account to pay from."))?;
    let available = account.available().map_or(0, Amount::minor);
    if available < price_minor {
        return Err(CmdError::msg(
            BUY_FUNDS
                .replace("{name}", &name)
                .replace("{price}", &price)
                .replace("{available}", &money_minor(ctx, available)),
        ));
    }
    if !crate::port::market::PortPartyCaps.has_cap(ctx.w, &listing.house, PartyCap::Accept) {
        tracing::error!(
            hull = hull.0,
            "tacsim: +salvage/buy: the house holds no acceptance cap"
        );
        return Err(CmdError::msg(SALE_FAILED));
    }
    let spec = sale_spec(account.id, &listing, ctx.actor, ctx.w.now());
    let id = mush_economy::offer_contract(ctx, spec).map_err(|error| {
        tracing::error!(%error, hull = hull.0, "tacsim: +salvage/buy: the sale was not offered");
        CmdError::msg(SALE_FAILED)
    })?;
    let unwind = |ctx: &mut Ctx<'_>, what: &str| {
        tracing::error!(
            contract = id.0,
            hull = hull.0,
            what,
            "tacsim: +salvage/buy: unwinding"
        );
        let _ = mush_economy::cancel_contract(ctx, id);
        ctx.out.pin_deferred();
        CmdError::msg(SALE_FAILED)
    };
    if !matches!(
        mush_economy::accept_contract(ctx, id),
        Ok(AcceptOutcome::Accepted { .. })
    ) {
        return Err(unwind(ctx, "the buyer's escrow was not placed"));
    }
    if !matches!(
        mush_economy::accept_contract_as(ctx, id, &listing.house),
        Ok(AcceptOutcome::Accepted {
            activated: true,
            ..
        })
    ) {
        return Err(unwind(
            ctx,
            "the house's acceptance did not activate the sale",
        ));
    }
    say(
        ctx,
        SALE_FILED
            .replace("{name}", &name)
            .replace("{house}", &house)
            .replace("{price}", &price),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_economy::{ContractRoles, EconomicVisibility, Expiry, TypedCondition};

    #[test]
    fn rs6c2_the_sale_contract_is_the_buyout_shape() {
        let listing = Listing {
            hull: ObjId(812),
            port: CanonicalId::new("tacsim.port.ceres").expect("id"),
            house: PrincipalId::Organization(mush_org::OrgId(7)),
            account: AccountId(70),
            price_minor: 1_234_500,
        };
        let spec = sale_spec(AccountId(5), &listing, ObjId(40), 1_000);
        assert_eq!(spec.parties, vec![listing.house.clone()]);
        assert_eq!(spec.considerations[0].account, AccountId(5));
        assert_eq!(spec.considerations[0].amount, Amount::from_minor(1_234_500));
        assert_eq!(spec.outcomes[0].payouts[0].to, AccountId(70));
        assert_eq!(spec.outcomes[0].asset_consequences, vec![0]);
        assert_eq!(
            spec.conditions[0],
            TypedCondition::TitleHeld {
                object: ObjId(812),
                by: listing.house.clone(),
            }
        );
        assert_eq!(
            spec.deliverables,
            vec![Deliverable::TitleTransfer {
                object: ObjId(812),
                from: listing.house.clone(),
                to: PrincipalId::Object(ObjId(40)),
            }]
        );
        assert_eq!(spec.deadline.map(|d| d.at), Some(1_000 + SALE_WINDOW_SECS));
        assert_eq!(spec.name, sale_name(ObjId(812)));
    }

    /// A contract `issuer` wrote, binding `parties`, delivering `deliverables`, in `state`,
    /// accepted (as it stands) by `accepted`.
    fn contract(
        issuer: &PrincipalId,
        parties: Vec<PrincipalId>,
        deliverables: Vec<Deliverable>,
        state: ContractState,
        accepted: &[&PrincipalId],
    ) -> EconomicContract {
        let consent = mush_economy::ConsentSurface {
            fact_consent: None,
            managed: None,
            parties,
            terms: mush_economy::ContractTerms {
                consideration: Vec::new(),
                deliverables,
                split: Vec::new(),
                remainder_to: AccountId(5),
                on_expiry: Expiry::Refund,
                outcomes: Vec::new(),
            },
            conditions: Vec::new(),
            deadline: None,
            visibility: EconomicVisibility::Parties,
            scope: None,
            adjudicator: None,
            evidence: Vec::new(),
        };
        let hash = mush_economy::consent_hash(issuer, &consent);
        EconomicContract {
            id: mush_economy::ContractId(9),
            issuer: issuer.clone(),
            name: sale_name(ObjId(812)),
            consent,
            consent_hash: hash,
            acceptances: accepted
                .iter()
                .map(|party| mush_economy::Acceptance {
                    party: (*party).clone(),
                    at: 1_000,
                    consent_hash: hash,
                })
                .collect(),
            satisfied: Vec::new(),
            escrow: Vec::new(),
            state,
            settle_attempts: 0,
            created_at: 1_000,
            amendments: 0,
            generation_started_at: 1_000,
            disputed_at: None,
            canon_published: None,
            roster_locked: false,
            deposits: Vec::new(),
            agreements: Vec::new(),
            pending: None,
            roles: ContractRoles::default(),
            real_deadline_extension_secs: 0,
            paused_real_remaining_secs: None,
            revision: 1,
        }
    }

    /// **Doc 48 §4.3** (the second review, doc 48b): a live sale is known by its shape. Every
    /// contract below carries the sale's own name; only the one the house accepted, live, that
    /// passes the hull's title from the house, is a sale.
    #[test]
    fn a_sale_is_its_shape_not_its_name() {
        let hull = ObjId(812);
        let house = PrincipalId::Organization(mush_org::OrgId(7));
        let buyer = PrincipalId::Object(ObjId(40));
        let title = |object: ObjId| Deliverable::TitleTransfer {
            object,
            from: house.clone(),
            to: buyer.clone(),
        };
        let both = vec![buyer.clone(), house.clone()];
        let sale = contract(
            &buyer,
            both.clone(),
            vec![title(hull)],
            ContractState::Active,
            &[&buyer, &house],
        );
        assert!(
            is_sale_of(&sale, hull, &house),
            "the buyer's sale, accepted"
        );
        // A player's bare draft under the sale's name: the triage's griefing case.
        let bare = contract(
            &buyer,
            vec![buyer.clone()],
            Vec::new(),
            ContractState::Draft,
            &[],
        );
        assert!(!is_sale_of(&bare, hull, &house), "a bare draft");
        // The sale's whole shape, still a draft.
        let draft = contract(
            &buyer,
            both.clone(),
            vec![title(hull)],
            ContractState::Draft,
            &[],
        );
        assert!(!is_sale_of(&draft, hull, &house), "a draft");
        // Offered to the house, which never accepted it.
        let offered = contract(
            &buyer,
            both.clone(),
            vec![title(hull)],
            ContractState::Offered,
            &[&buyer],
        );
        assert!(
            !is_sale_of(&offered, hull, &house),
            "unaccepted by the house"
        );
        // The house's acceptance bound to another consent is no acceptance.
        let mut stale = sale.clone();
        for acceptance in &mut stale.acceptances {
            acceptance.consent_hash = [0; 32];
        }
        assert!(!is_sale_of(&stale, hull, &house), "a void acceptance");
        // Another hull's sale, and a finished one.
        let other = contract(
            &buyer,
            both.clone(),
            vec![title(ObjId(813))],
            ContractState::Active,
            &[&buyer, &house],
        );
        assert!(!is_sale_of(&other, hull, &house), "another hull");
        let mut done = sale.clone();
        done.state = ContractState::Cancelled;
        assert!(!is_sale_of(&done, hull, &house), "terminal");
        // Another house's sale of her title is not this house's.
        let elsewhere = PrincipalId::Organization(mush_org::OrgId(8));
        assert!(!is_sale_of(&sale, hull, &elsewhere), "another house");
    }
}
