//! 17g C1b: the one place a format-2 deployment's numbers become the player
//! market's typed policies. Decode builds them once (`LaunchDeployment::validate`)
//! so each policy constructor's own checks run, and C1c composes the market from
//! them in the launch's Active branch. Decode does not prove the rest: the
//! entitlement policy's own validation (`EntitlementConfig::new`) and the owner
//! configs that wrap these policies (`OriginalQuoteConfig::new`,
//! `RenewalConfig::new`, `PaidResaleConfig::new`) run only at Active
//! composition, which refuses there. Nothing here reads the world or grants a
//! right.
//!
//! What the deployment does not choose (C1 plan §6):
//! - the personal term and Grace, and the Registry levy, are doc 17's
//!   (`content::TERM_SECONDS`, `GRACE_SECONDS`, `LEVY_PERMILLE`; 17f §2.3
//!   "come from doc 17");
//! - the personal venue cut is 0 with no recipient (17f §2.3, a direct author
//!   listing), for purchases and renewals alike;
//! - personal licences are transferable, policy-wide (D-UM-40); the author's
//!   per-listing non-transferable mark (17g C2b) only narrows it, listing by
//!   listing (`@mind/publish ... transfer=no`, 17g C2c).
use super::{
    LaunchDeployment,
    content::{self, bad},
};
use mush_core::{ModError, ObjId};
use mush_economy::{Amount, InstrumentId, managed_entitlement::GrossRecipientCut};
use mush_market::{
    mind_entitlement::EntitlementPolicy,
    mind_renewal::RenewalPolicy,
    mind_trade::{OriginalDuration, OriginalPrice, OriginalQuotePolicy, records::QuoteVenue},
};
use mush_org::PrincipalId;
use mush_roll::commerce::{
    PendingCharterTransferConfig, registry_inspection::RegistryInspectionPolicy,
    registry_recall::RegistryRecallPolicy, registry_revocation::RegistryRevocationPolicy,
};

/// 17g C1b: every player-market policy a format-2 deployment names, built and
/// checked by its owner's own constructor. `entitlement` is the exception: its
/// validation is private to mush-market and runs in `EntitlementConfig::new` at
/// Active composition. Decode bounds its numeric fields
/// (`LaunchDeployment::validate`) but does not prove it. 17g C1c's Active
/// branch of `LaunchComposition::prepare` composes the market from them.
pub(crate) struct MarketPolicies {
    /// The v2 original quote policy (`with_original_source_purchases`).
    pub(crate) quote: OriginalQuotePolicy,
    /// The v3 entitlement policy (`with_mind_entitlements`).
    pub(crate) entitlement: EntitlementPolicy,
    /// The personal renewal fee policy (`with_mind_renewals`).
    pub(crate) renewal: RenewalPolicy,
    /// The pending-charter service actor and settlement window
    /// (`with_pending_charter_transfers`).
    pub(crate) pending: PendingCharterTransferConfig,
    /// The paid-resale service actor (`with_paid_mind_resales`), the same clerk.
    pub(crate) clerk: ObjId,
    /// The paid-resale offer window, in real seconds (`with_paid_mind_resales`).
    pub(crate) resale_offer_seconds: i64,
    /// The Registry recall scope and operators (`with_registry_recall`).
    pub(crate) recall: RegistryRecallPolicy,
    /// The Registry revocation scope and operators (`with_registry_revocation`).
    pub(crate) revocation: RegistryRevocationPolicy,
    /// Doc 17h cut M8a: the Registry inspection scope and operators
    /// (`with_registry_inspection`), the same authority.
    pub(crate) inspection: RegistryInspectionPolicy,
    /// Doc 17h cut M8d D3: the levy sink a Registry fine pays (`with_registry_fines`), the
    /// deployment's `registry` recipient.
    pub(crate) fines: mush_economy::AccountId,
}

/// 17g C1b: `None` for a format-1 deployment (no market), the eight policies
/// for a format-2 one. Every owner refusal is mapped explicitly, so a refused
/// value never panics or passes. The caller decides the message a refusal
/// becomes (`LaunchDeployment::validate` names the player market).
pub(crate) fn policies(d: &LaunchDeployment) -> Result<Option<MarketPolicies>, ModError> {
    let Some(market) = &d.market else {
        return Ok(None);
    };
    let terms = &market.terms;
    let authority = &market.registry_authority;
    let credit = InstrumentId::new(crate::economy::CREDIT).ok_or_else(|| bad("credit identity"))?;
    // The personal price policy pays its levy to a system principal.
    let PrincipalId::System(registry) = &d.registry.owner else {
        return Err(bad("personal Registry recipient is not a system principal"));
    };
    let price = Amount::from_minor(terms.price_minor);
    let renewal_price = Amount::from_minor(terms.renewal_price_minor);
    let levy = GrossRecipientCut {
        permille: content::LEVY_PERMILLE,
        recipient: Some(d.registry.clone()),
    };
    let quote = OriginalQuotePolicy::new(
        OriginalPrice::new(
            price,
            credit.clone(),
            registry.clone(),
            d.registry.account,
            content::LEVY_PERMILLE,
        )?,
        QuoteVenue::new(
            d.venue.name.clone(),
            d.venue.recipient.owner.clone(),
            d.venue.recipient.account,
            0,
        )
        .map_err(ModError::msg)?,
        OriginalDuration::new(
            true,
            content::TERM_SECONDS,
            content::GRACE_SECONDS,
            terms.quote_seconds,
        )?,
    )?;
    let entitlement = EntitlementPolicy {
        price,
        instrument: credit,
        registry: levy.clone(),
        venue_name: d.venue.name.clone(),
        venue: GrossRecipientCut {
            permille: 0,
            recipient: None,
        },
        transferable: true,
        term_seconds: content::TERM_SECONDS,
        grace_seconds: content::GRACE_SECONDS,
        quote_seconds: terms.quote_seconds,
        renewal_price,
        renewal_term_seconds: content::TERM_SECONDS,
        renewal_grace_seconds: content::GRACE_SECONDS,
    };
    let renewal = RenewalPolicy {
        registry: levy,
        venue: None,
        offer_ic_seconds: terms.renewal_offer_ic_seconds,
    };
    renewal.validate()?;
    let pending = PendingCharterTransferConfig::new(market.clerk, terms.charter_settle_ic_seconds)?;
    let recall = RegistryRecallPolicy::new(
        d.registry_issuer.clone(),
        authority.organization,
        authority.scope,
        authority.operators.clone(),
    )?;
    let revocation = RegistryRevocationPolicy::new(
        d.registry_issuer.clone(),
        authority.organization,
        authority.scope,
        authority.operators.clone(),
    )?;
    let inspection = RegistryInspectionPolicy::new(
        d.registry_issuer.clone(),
        authority.organization,
        authority.scope,
        authority.operators.clone(),
    )?;
    Ok(Some(MarketPolicies {
        quote,
        entitlement,
        renewal,
        pending,
        clerk: market.clerk,
        resale_offer_seconds: terms.resale_offer_seconds,
        recall,
        revocation,
        inspection,
        fines: d.registry.account,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::{
        HardwareCoordinates, HouseDeployment, LaunchMode, LaunchVenue, MarketTerms, NamedHouses,
        PlayerMarket, RegistryAuthority,
    };
    use super::*;
    use mush_economy::managed_entitlement::Recipient;
    use mush_economy::{AccountId, VaultId, core_integration::ScopedCoreAccount};
    use mush_industry::core_production::OriginAgent;
    use mush_inventory::Grade;
    use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
    use serde_json::{Value, json};

    type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

    fn system(name: &str) -> Result<SystemPrincipalId> {
        Ok(SystemPrincipalId::new(name).ok_or("system principal")?)
    }
    fn house(n: u64, name: &str) -> HouseDeployment {
        HouseDeployment {
            organization: OrgId(n),
            scope: OrgScopeId(n),
            treasury: AccountId(n),
            publisher: ObjId(2),
            source_name: name.into(),
            attestation_agent: None,
        }
    }
    /// A format-1 deployment with illustrative ids; decode never reads a world.
    fn format_one() -> Result<LaunchDeployment> {
        Ok(LaunchDeployment {
            format: 1,
            mode: LaunchMode::Initialize,
            registry_issuer: system("launch.issuer")?,
            registry: Recipient {
                account: AccountId(20),
                owner: PrincipalId::System(system("launch.registry")?),
            },
            venue: LaunchVenue {
                name: mush_market::MarketId::new("launch.counter").ok_or("market")?,
                room: ObjId(30),
                recipient: Recipient {
                    account: AccountId(21),
                    owner: PrincipalId::System(system("launch.venue")?),
                },
            },
            houses: NamedHouses {
                meridiani: house(1, "Meridiani Field Series"),
                coprates: house(2, "Coprates Drill"),
                tycho: house(3, "Safe Conduct"),
            },
            republic: mush_faction::FactionId(1),
            hardware: HardwareCoordinates {
                importer: OrgId(4),
                stock_scope: OrgScopeId(4),
                depot: VaultId(1),
                agents: [
                    OriginAgent {
                        actor: ObjId(2),
                        stock: VaultId(2),
                    },
                    OriginAgent {
                        actor: ObjId(3),
                        stock: VaultId(3),
                    },
                ],
                grades: [Grade::new(10)?, Grade::new(20)?, Grade::new(30)?],
                supplier_account: AccountId(4),
                integrator: ScopedCoreAccount {
                    account: AccountId(5),
                    organization: OrgId(4),
                    scope: OrgScopeId(5),
                },
                factory: ObjId(31),
                output_container: ObjId(32),
                ungovern: None,
            },
            market: None,
            procurement: Vec::new(),
        })
    }
    fn format_two() -> Result<LaunchDeployment> {
        let mut d = format_one()?;
        d.format = 2;
        d.market = Some(PlayerMarket {
            clerk: ObjId(40),
            registry_authority: RegistryAuthority {
                organization: OrgId(6),
                scope: OrgScopeId(6),
                operators: vec![ObjId(41)],
            },
            terms: MarketTerms::PROVISIONAL,
        });
        Ok(d)
    }

    /// Doc 17h §3 M8f (P2): a deployment's procurement policies decode only on format 2, each
    /// naming a distinct named House and a distinct agent that is neither the market's clerk
    /// nor a Registry operator, inside its own bounds; empty, the deployment serializes without
    /// the key (its `launch_deployment` descriptor does not move).
    #[test]
    fn procurement_policies_decode_only_where_they_can_buy() -> Result {
        use super::super::{ProcurementPolicy, ProcurementRole};
        let policy = |org: u64, agent: u32| ProcurementPolicy {
            org: OrgId(org),
            scope: OrgScopeId(org),
            agent: ObjId(agent),
            budget_minor_per_ic_month: None,
            standing: None,
            cap_permille: None,
            roles: ProcurementRole::all(),
        };
        let bare = format_two()?;
        check!(
            serde_json::to_value(&bare)?.get("procurement").is_none(),
            "no policy, no key"
        );
        let mut d = format_two()?;
        d.procurement = vec![policy(2, 60), policy(1, 61)];
        d.validate()?;
        let decoded = LaunchDeployment::decode(&serde_json::to_vec(&d)?)?;
        check_eq!(decoded.procurement, d.procurement);
        let mut one = format_one()?;
        one.procurement = vec![policy(2, 60)];
        check!(
            one.validate().is_err(),
            "format 1 has no market to buy from"
        );
        for (policies, why) in [
            (vec![policy(9, 60)], "not a named House"),
            (vec![policy(2, 60), policy(2, 61)], "one House twice"),
            (vec![policy(2, 60), policy(1, 60)], "one agent twice"),
            (vec![policy(2, 40)], "the market's clerk"),
            (vec![policy(2, 41)], "a Registry operator"),
            (vec![policy(2, 0)], "agent #0"),
            (
                vec![ProcurementPolicy {
                    cap_permille: Some(0),
                    ..policy(2, 60)
                }],
                "a cap of zero",
            ),
        ] {
            let mut d = format_two()?;
            d.procurement = policies;
            check!(d.validate().is_err(), "{why}");
        }
        Ok(())
    }

    #[test]
    fn format_one_has_no_market_policies() -> Result {
        let d = format_one()?;
        d.validate()?;
        check!(policies(&d)?.is_none());
        Ok(())
    }

    #[test]
    fn provisional_terms_build_every_policy() -> Result {
        let d = format_two()?;
        d.validate()?;
        let p = policies(&d)?.ok_or("format 2 builds the market policies")?;
        let levy = GrossRecipientCut {
            permille: content::LEVY_PERMILLE,
            recipient: Some(d.registry.clone()),
        };
        let no_venue = GrossRecipientCut {
            permille: 0,
            recipient: None,
        };
        // The v2 quote policy: the price, the levy to the deployment's
        // Registry recipient, the counter's name with a 0 cut, transferable,
        // the content term and Grace, and the quote window. Its fields are
        // private, so its serialized form is read.
        let quote = serde_json::to_value(&p.quote)?;
        check_eq!(
            quote,
            json!({
                "price": serde_json::to_value(Amount::from_minor(40_000))?,
                "instrument": crate::economy::CREDIT,
                "registry_recipient": "launch.registry",
                "registry_account": 20,
                "registry_permille": content::LEVY_PERMILLE,
                "venue": {
                    "name": "launch.counter",
                    "recipient": {"System": "launch.venue"},
                    "account": 21,
                    "permille": 0,
                },
                "transferable": true,
                "term_seconds": content::TERM_SECONDS,
                "grace_seconds": content::GRACE_SECONDS,
                "quote_seconds": 300,
            })
        );
        // The v3 entitlement policy.
        let e = &p.entitlement;
        check_eq!(e.price, Amount::from_minor(40_000));
        check_eq!(e.renewal_price, Amount::from_minor(40_000));
        check_eq!(e.instrument.as_str(), crate::economy::CREDIT);
        check_eq!(&e.registry, &levy);
        check_eq!(&e.venue, &no_venue);
        check_eq!(e.venue_name.as_str(), "launch.counter");
        check!(e.transferable);
        check_eq!(
            (e.term_seconds, e.grace_seconds),
            (content::TERM_SECONDS, content::GRACE_SECONDS)
        );
        check_eq!(
            (e.renewal_term_seconds, e.renewal_grace_seconds),
            (content::TERM_SECONDS, content::GRACE_SECONDS)
        );
        check_eq!(e.quote_seconds, 300);
        // The personal renewal fee policy: the same levy, no venue.
        check_eq!(&p.renewal.registry, &levy);
        check!(p.renewal.venue.is_none());
        check_eq!(p.renewal.offer_ic_seconds, 259_200);
        // The pending-charter config: the clerk, and the settle window. Its
        // fields are crate-private to mush-roll, so it is compared whole.
        check_eq!(
            format!("{:?}", p.pending),
            format!(
                "{:?}",
                PendingCharterTransferConfig::new(ObjId(40), 259_200)?
            )
        );
        check_eq!(p.clerk, ObjId(40));
        check_eq!(p.resale_offer_seconds, 86_400);
        // The Registry authority, both twins on the same scope and operators.
        check_eq!(
            p.recall,
            RegistryRecallPolicy::new(
                system("launch.issuer")?,
                OrgId(6),
                OrgScopeId(6),
                vec![ObjId(41)]
            )?
        );
        check_eq!(
            p.revocation,
            RegistryRevocationPolicy::new(
                system("launch.issuer")?,
                OrgId(6),
                OrgScopeId(6),
                vec![ObjId(41)]
            )?
        );
        // Doc 17h cut M8a: the inspection, the third twin on the same authority.
        check_eq!(
            p.inspection,
            RegistryInspectionPolicy::new(
                system("launch.issuer")?,
                OrgId(6),
                OrgScopeId(6),
                vec![ObjId(41)]
            )?
        );
        // The format-2 document round-trips through decode.
        let bytes = serde_json::to_vec(&d)?;
        let back = LaunchDeployment::decode(&bytes)?;
        check_eq!(
            serde_json::to_value(&back)?,
            serde_json::from_slice::<Value>(&bytes)?
        );
        Ok(())
    }

    /// The provisional terms pair their values (both prices 40_000, both IC
    /// windows 259_200), so `provisional_terms_build_every_policy` cannot see a
    /// swapped field. Six pairwise-distinct terms, each inside its bound, must
    /// each land in its own policy field: the pending window is the settle
    /// window, not the renewal offer window, and the purchase price is not the
    /// renewal price (17g §1 ruling 6 tunes them apart in play).
    #[test]
    fn distinct_terms_land_in_their_own_policy_fields() -> Result {
        let mut d = format_two()?;
        let terms = MarketTerms {
            price_minor: 40_001,
            renewal_price_minor: 40_002,
            quote_seconds: 299,
            renewal_offer_ic_seconds: 172_800,
            resale_offer_seconds: 86_399,
            charter_settle_ic_seconds: 259_199,
        };
        if let Some(market) = d.market.as_mut() {
            market.terms = terms.clone();
        }
        d.validate()?;
        let p = policies(&d)?.ok_or("format 2 builds the market policies")?;
        let quote = serde_json::to_value(&p.quote)?;
        check_eq!(
            quote.get("price"),
            Some(&serde_json::to_value(Amount::from_minor(40_001))?)
        );
        check_eq!(quote.get("quote_seconds"), Some(&json!(299)));
        check_eq!(p.entitlement.price, Amount::from_minor(40_001));
        check_eq!(p.entitlement.renewal_price, Amount::from_minor(40_002));
        check_eq!(p.entitlement.quote_seconds, 299);
        check_eq!(p.renewal.offer_ic_seconds, 172_800);
        check_eq!(
            format!("{:?}", p.pending),
            format!(
                "{:?}",
                PendingCharterTransferConfig::new(ObjId(40), 259_199)?
            )
        );
        check!(
            format!("{:?}", p.pending)
                != format!(
                    "{:?}",
                    PendingCharterTransferConfig::new(ObjId(40), 172_800)?
                )
        );
        check_eq!(p.resale_offer_seconds, 86_399);
        // The document carries the same six values back.
        let back = LaunchDeployment::decode(&serde_json::to_vec(&d)?)?;
        check_eq!(back.market.map(|m| m.terms), Some(terms));
        Ok(())
    }
}
