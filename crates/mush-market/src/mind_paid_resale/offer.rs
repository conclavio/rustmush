//! Seller-authored price and named buyer; actual accepted issuance supplies cuts.
use super::{accounts, records::MindPaidResaleOffer};
use mush_core::{ActionTxn, CommitToken, World};
use mush_economy::managed_entitlement::Recipient;
use mush_economy::managed_paid_resale::{
    MAX_OFFER_SECONDS, PaidResaleTermsV1, SETTLEMENT_IC_SECONDS,
};
use mush_economy::{AccountId, Amount};
use mush_roll::commerce::paid_resale::{ResaleRates, protocol, reader::PaidResaleFacts};

pub(super) fn build(
    world: &World,
    txn: &mut ActionTxn<'_>,
    legacy: Option<&crate::mind_trade::Families>,
    facts: &PaidResaleFacts,
    operation: CommitToken,
    price: Amount,
    seller_account: AccountId,
    seconds: i64,
) -> Result<MindPaidResaleOffer, String> {
    if facts.requester() != facts.seller()
        || price.minor() <= 0
        || !(1..=MAX_OFFER_SECONDS).contains(&seconds)
    {
        return Err("paid resale offer differs".into());
    }
    let e = accounts::live(world)?;
    let accepted = facts.economics();
    let (registry, venue, rates) = match accepted.rates() {
        ResaleRates::Legacy {
            quote,
            quote_digest,
        } => {
            let actual = legacy
                .ok_or("retained legacy quote family unavailable")?
                .resale_quote(txn, *quote)?;
            if actual.digest()? != *quote_digest {
                return Err("retained legacy quote differs".into());
            }
            let (instrument, registry, venue) = actual.resale_rates()?;
            if &instrument != accepted.instrument() {
                return Err("accepted instrument differs".into());
            }
            (registry, venue, Vec::new())
        }
        ResaleRates::Accepted {
            registry,
            venue,
            parents,
        } => (registry.clone(), venue.clone(), parents.clone()),
    };
    accounts::cuts(e, accepted.instrument(), &registry, venue.as_ref())?;
    accounts::personal(
        world,
        e,
        accepted.instrument(),
        facts.seller(),
        seller_account,
    )?;
    let parents = accounts::parents(world, e, accepted.instrument(), &rates)?;
    let row = MindPaidResaleOffer {
        format: 1,
        operation,
        action: protocol::actions().map_err(|e| e.to_string())?[0].clone(),
        binding: facts.binding().clone(),
        terms: PaidResaleTermsV1 {
            format: 1,
            offer: operation,
            offer_action: protocol::actions().map_err(|e| e.to_string())?[0].clone(),
            basis: facts.binding().native_basis().map_err(|e| e.to_string())?,
            seller: facts.seller(),
            buyer: facts.buyer(),
            instrument: accepted.instrument().clone(),
            price,
            registry,
            venue,
            parents,
            remainder: Recipient {
                account: seller_account,
                owner: mush_org::PrincipalId::Object(facts.seller()),
            },
            offered_at_ic: world.world_ic_now(),
            offered_at_utc: world.now(),
            expires_at_utc: world
                .now()
                .checked_add(seconds)
                .ok_or("offer expiry overflow")?,
            settle_seconds: SETTLEMENT_IC_SECONDS,
        },
    };
    row.validate()?;
    Ok(row)
}
