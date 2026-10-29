//! Strict host coordinates. Decoding config creates no world object or right.
//!
//! Format 1 names the House chain's identities only. Format 2 (17g C1b) adds
//! [`PlayerMarket`]: the player market's world identities and its provisional
//! tuning (17g §1 ruling 6). Decoding format 2 builds every typed market policy
//! once (`super::market::policies`) and still creates no object, grant or
//! account. Boot checks that the named identities exist (`Registration`), and
//! C1c composes the market from them.
use mush_core::{GOD, ModError, ObjId};
use mush_economy::{
    AccountId, VaultId, core_integration::ScopedCoreAccount, managed_entitlement::Recipient,
};
use mush_faction::FactionId;
use mush_industry::core_production::OriginAgent;
use mush_inventory::Grade;
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use serde::{Deserialize, Serialize};

pub const MAX_DEPLOYMENT_BYTES: usize = 65_536;
/// 17g C1b: the decode ceiling of a personal Pin price and of its renewal
/// price, in minor credits (1,000,000cr). A bound on an operator's typo, not a
/// tuning value (C1 plan §6).
const MAX_MARKET_PRICE_MINOR: i64 = 100_000_000;
/// The Registry recall and revocation policies' own operator bound.
const MAX_REGISTRY_OPERATORS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    Initialize,
    Active,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HouseDeployment {
    pub organization: OrgId,
    pub scope: OrgScopeId,
    pub treasury: AccountId,
    pub publisher: ObjId,
    pub source_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedHouses {
    pub meridiani: HouseDeployment,
    pub coprates: HouseDeployment,
    pub tycho: HouseDeployment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchVenue {
    pub name: mush_market::MarketId,
    pub room: ObjId,
    pub recipient: Recipient,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareCoordinates {
    pub importer: OrgId,
    pub stock_scope: OrgScopeId,
    pub depot: VaultId,
    pub agents: [OriginAgent; 2],
    pub grades: [Grade; 3],
    pub supplier_account: AccountId,
    pub integrator: ScopedCoreAccount,
    pub factory: ObjId,
    pub output_container: ObjId,
}
impl HardwareCoordinates {
    pub(super) fn deployment(
        &self,
        issuer: &SystemPrincipalId,
    ) -> crate::hardware::HardwareDeployment {
        crate::hardware::HardwareDeployment {
            importer: self.importer,
            stock_scope: self.stock_scope,
            depot: self.depot,
            agents: self.agents.clone(),
            grades: self.grades,
            supplier_account: self.supplier_account,
            integrator: self.integrator.clone(),
            issuer: issuer.clone(),
            factory: self.factory,
            output_container: self.output_container,
            // Doc 17h cut M8b C2: the launch composes no ungoverning shop.
            ungovern: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchDeployment {
    /// 1: the House chain only. 2 (17g C1b): with the player market, which
    /// `market` then names. Any other value is refused at decode.
    pub format: u16,
    pub mode: LaunchMode,
    pub registry_issuer: SystemPrincipalId,
    pub registry: Recipient,
    pub venue: LaunchVenue,
    pub houses: NamedHouses,
    pub republic: FactionId,
    pub hardware: HardwareCoordinates,
    /// 17g C1b: present exactly when `format` is 2. It is skipped when absent,
    /// so a format-1 document serializes to exactly its eight pre-C1 keys, and
    /// the `launch_deployment` descriptor of a format-1 world (`Registration::new`:
    /// the deployment minus `mode`) does not move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<PlayerMarket>,
}

/// 17g C1b: the player market's world identities and tuning (format 2).
/// Decoding it proves shape and bounds only. Boot checks that the identities
/// exist and are of the right kind; every grant and status stays a runtime fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerMarket {
    /// The live character that runs pending-charter settlement and paid-resale
    /// terminal steps (`PendingCharterTransferConfig`'s service actor,
    /// `with_paid_mind_resales`'s service). Never #0 and never #1.
    pub clerk: ObjId,
    pub registry_authority: RegistryAuthority,
    pub terms: MarketTerms,
}

/// 17g C1b: the Registry organization, scope and operators that recall a filed
/// revision and revoke a term. Naming an operator here grants nothing. The
/// operator still needs both capabilities by `@org/grant` on this exact scope,
/// and the providers authorize every offer and invocation live.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryAuthority {
    /// The Registry organization: never a House organization or the importer.
    pub organization: OrgId,
    /// The scope of that organization which the operators' grants name.
    pub scope: OrgScopeId,
    /// 1..=32, strictly ascending (the Registry recall and revocation policies'
    /// own rule), none of them #0.
    pub operators: Vec<ObjId>,
}

/// 17g C1b: the player market's provisional tuning (17g §1 ruling 6). The
/// personal term and Grace, the Registry levy and the filing fee are not here:
/// they are doc 17's and stay the content constants (17f §2.3). The personal
/// venue cut (0) and transferability (D-UM-40) are fixed in
/// `super::market::policies`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketTerms {
    /// A personal Pin's price, original or derived (one policy-wide price as
    /// built), in minor credits. 1..=100,000,000.
    pub price_minor: i64,
    /// A personal Pin renewal's price, in minor credits. 1..=100,000,000.
    pub renewal_price_minor: i64,
    /// How long a personal purchase quote stands, in real seconds.
    /// 1..=`MAX_QUOTE_SECONDS` (600).
    pub quote_seconds: i64,
    /// How long a personal renewal offer stands, in IC seconds.
    /// 1..=`MAX_RENEWAL_OFFER_IC_SECONDS` (3 IC days).
    pub renewal_offer_ic_seconds: i64,
    /// How long a paid-resale offer stands, in real seconds.
    /// 1..=`MAX_OFFER_SECONDS` (one real day).
    pub resale_offer_seconds: i64,
    /// The pending-charter settlement window, in IC seconds (D-UM-40's
    /// `transfer_settle_ic_days`, Q-UM-18).
    /// 1..=`MAX_CHARTER_SETTLEMENT_IC_SECONDS` (3 IC days).
    pub charter_settle_ic_seconds: i64,
}
impl MarketTerms {
    /// 17g §1 ruling 6: provisional launch tuning, tuned in play.
    pub const PROVISIONAL: Self = Self {
        // 400cr, the House Pin's price (17f §2.3).
        price_minor: 40_000,
        // 400cr.
        renewal_price_minor: 40_000,
        // The House quote's window.
        quote_seconds: 300,
        // 3 IC days (17f §2.3).
        renewal_offer_ic_seconds: 259_200,
        // One real day, `MAX_OFFER_SECONDS`.
        resale_offer_seconds: 86_400,
        // 3 IC days (D-UM-40, Q-UM-18's default).
        charter_settle_ic_seconds: 259_200,
    };
}
impl LaunchDeployment {
    pub fn decode(bytes: &[u8]) -> Result<Self, ModError> {
        if bytes.is_empty() || bytes.len() > MAX_DEPLOYMENT_BYTES {
            return Err(ModError::msg(
                "launch deployment exceeds its bounded document",
            ));
        }
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|_| ModError::msg("invalid launch deployment document"))?;
        value.validate()?;
        Ok(value)
    }
    pub(super) fn validate(&self) -> Result<(), ModError> {
        let houses = [
            &self.houses.meridiani,
            &self.houses.coprates,
            &self.houses.tycho,
        ];
        // 17g C1b: format 1 forbids a market and format 2 requires one; any
        // other format is unknown.
        let formatted = matches!((self.format, &self.market), (1, None) | (2, Some(_)));
        if !formatted || self.registry.account == self.venue.recipient.account {
            return Err(ModError::msg("launch deployment identity differs"));
        }
        for (index, house) in houses.iter().enumerate() {
            if mush_roll::canonical_name(&house.source_name).is_err()
                || house.source_name.contains('=')
                || houses.iter().take(index).any(|other| {
                    other.organization == house.organization
                        || other.treasury == house.treasury
                        || (other.publisher == house.publisher
                            && other.source_name.eq_ignore_ascii_case(&house.source_name))
                })
            {
                return Err(ModError::msg(
                    "named launch houses must have distinct exact identities",
                ));
            }
        }
        if let Some(market) = &self.market
            && !self.market_holds(market, &houses)
        {
            return Err(ModError::msg("launch deployment player market differs"));
        }
        Ok(())
    }
    /// 17g C1b: format 2's shape and bounds, and every typed policy built once.
    /// - The Registry recipient is a system principal (the personal price
    ///   policy's Registry recipient is one).
    /// - The clerk is neither #0 nor GOD.
    /// - The operators obey the Registry policies' own rule, and none is #0.
    /// - The Registry organization is its own: no House's, not the importer.
    /// - Every term is inside its bound: the owners' window bounds, and the
    ///   deployment's own price ceiling (`MAX_MARKET_PRICE_MINOR`).
    ///
    /// `EntitlementPolicy`'s own validation is private and runs only at Active
    /// composition (`EntitlementConfig::new`). These bounds cover its numeric
    /// fields; decode does not prove the rest, which that boot refuses.
    fn market_holds(&self, market: &PlayerMarket, houses: &[&HouseDeployment; 3]) -> bool {
        use mush_economy::{
            managed_charter_transfer::MAX_CHARTER_SETTLEMENT_IC_SECONDS,
            managed_paid_resale::MAX_OFFER_SECONDS, managed_renewal::MAX_RENEWAL_OFFER_IC_SECONDS,
        };
        use mush_market::mind_trade::records::MAX_QUOTE_SECONDS;
        let authority = &market.registry_authority;
        let terms = &market.terms;
        let operators = &authority.operators;
        matches!(self.registry.owner, mush_org::PrincipalId::System(_))
            && market.clerk != ObjId(0)
            && market.clerk != GOD
            && (1..=MAX_REGISTRY_OPERATORS).contains(&operators.len())
            && operators
                .windows(2)
                .all(|pair| matches!(pair, [a, b] if a < b))
            && !operators.contains(&ObjId(0))
            && houses
                .iter()
                .all(|house| house.organization != authority.organization)
            && authority.organization != self.hardware.importer
            && (1..=MAX_MARKET_PRICE_MINOR).contains(&terms.price_minor)
            && (1..=MAX_MARKET_PRICE_MINOR).contains(&terms.renewal_price_minor)
            && (1..=MAX_QUOTE_SECONDS).contains(&terms.quote_seconds)
            && (1..=MAX_RENEWAL_OFFER_IC_SECONDS).contains(&terms.renewal_offer_ic_seconds)
            && (1..=MAX_OFFER_SECONDS).contains(&terms.resale_offer_seconds)
            && (1..=MAX_CHARTER_SETTLEMENT_IC_SECONDS).contains(&terms.charter_settle_ic_seconds)
            && super::market::policies(self).is_ok_and(|policies| policies.is_some())
    }
}
