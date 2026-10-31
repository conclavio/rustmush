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
    /// Doc 17h §3 M8h (A1): the character the `tacsim.house_attestation` tick attests this
    /// House's filings as, where the House has no procurement policy (a policy's agent wins,
    /// and a different one named here refuses the boot). A live, non-GOING Player-kind
    /// character on an operator-held login, granted `roll.holder.org` on `scope` (the runbook);
    /// absent, and with no policy, the House's filings are not attested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_agent: Option<ObjId>,
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
    /// **An ungoverning shop** (doc 17h cut M8g, the lead's ruling (a)): a facility holding
    /// `core_ungovern` and the provider it pays, composed as the hardware's
    /// `UngoverningSite` (doc 17h cut M8b C2). Skipped when absent, so every shipped
    /// deployment, its `launch_deployment` identity and its digest are as before: the launch
    /// composes no ungoverning shop (doc 17h §4) unless a deployment names one, as the combined
    /// M8 test world does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ungovern: Option<UngoverningCoordinates>,
    /// **A refit bay** (doc 31 cut UD.4b): a facility holding `bay_refit`, its stores and the
    /// provider it pays. Skipped when absent, so every deployment that names none, its
    /// `launch_deployment` identity and its digest are as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bay: Option<BayCoordinates>,
}

/// **A refit bay's coordinates** (doc 31 cut UD.4b): the deployment's serializable twin of
/// `crate::hardware::RefitBay`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayCoordinates {
    /// The bay's organisation, scope and treasury, which the fee pays.
    pub provider: ScopedCoreAccount,
    /// The facility holding `bay_refit`.
    pub facility: ObjId,
    /// The bay's stores: the vault parts come from and go back to.
    pub stores: VaultId,
    /// The highest fee a quote may charge, centicredits; absent, the hardware default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fee_minor: Option<i64>,
}

/// **The ungoverning shop's coordinates** (doc 17h cut M8g, ruling (a)): the deployment's
/// serializable twin of `crate::hardware::UngoverningSite`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UngoverningCoordinates {
    /// The shop's organisation, scope and treasury, which the fee pays.
    pub provider: ScopedCoreAccount,
    /// The facility holding `core_ungovern` (and not `core_refit`).
    pub factory: ObjId,
    /// The shop's own completed-work container.
    pub output_container: ObjId,
    /// The shop's fee in centicredits; absent, the catalogue's `ungovern_minor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_minor: Option<i64>,
}
impl HardwareCoordinates {
    pub(crate) fn deployment(
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
            // Doc 17h cut M8b C2: the launch composes no ungoverning shop unless the deployment
            // names one (doc 17h cut M8g, ruling (a)).
            ungovern: self
                .ungovern
                .as_ref()
                .map(|u| crate::hardware::UngoverningSite {
                    provider: u.provider.clone(),
                    factory: u.factory,
                    output_container: u.output_container,
                    fee_minor: u.fee_minor,
                }),
            bay: self.bay.as_ref().map(|b| crate::hardware::RefitBay {
                provider: b.provider.clone(),
                facility: b.facility,
                stores: b.stores,
                max_fee_minor: b.max_fee_minor,
            }),
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
    /// Doc 17h §3 M8f (P2): the named Houses' procurement policies, at most one per House,
    /// format 2 only. Skipped when empty, so a deployment without procurement serializes, and
    /// its `launch_deployment` descriptor reads, exactly as before M8f.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procurement: Vec<ProcurementPolicy>,
}

/// Doc 17h §3 M8f (P2): which listing forms a procurement policy may buy (the player market's
/// three: `sealed`, `open`, `open_resale`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcurementRole {
    Sealed,
    Open,
    OpenResale,
}
impl ProcurementRole {
    /// Every form, the default.
    #[must_use]
    pub fn all() -> Vec<Self> {
        vec![Self::Sealed, Self::Open, Self::OpenResale]
    }
}

/// Doc 17h §3 M8f (the lead's rulings P2, P4): one named House's procurement policy. The House
/// buys mind licences from the player market's listings, once per IC calendar month, through
/// one named acting character, its **procurement agent**: an NPC character the operator creates
/// (a Player-kind object), stands at the market's venue, and grants `roll.holder.org` on the
/// organisation's root scope **with a limit of the monthly budget** (`@org/grant <agent>=<org>
/// roll.holder.org limit=<budget>`), so the grant is the second safety beside the ledger's
/// monthly cap. Naming the agent here grants nothing (the Registry operators' precedent): every
/// buy re-asks the grant live. `standing`, when set, is the House's backing faction: the agent
/// is made a member of it, so the per-character standing rules admit the House's agent (Q-M8-4,
/// the lead's call; no new standing predicate). Budget and cap left out take
/// `[tacsim.procurement]`'s defaults (`crate::config::ProcurementConfig`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcurementPolicy {
    /// One of the three named Houses' organisations.
    pub org: OrgId,
    /// The organisation's ROOT scope: the scope its treasury pays a buy from and the agent's
    /// grant names (M8c F5). Boot checks it is the root.
    pub scope: OrgScopeId,
    /// The House's procurement agent: a live, non-GOING Player-kind character, never #0 or GOD.
    pub agent: ObjId,
    /// Minor credits per IC calendar month; absent, `[tacsim.procurement]`'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_minor_per_ic_month: Option<i64>,
    /// The House's backing faction (Q-M8-4); absent, it buys only lines no standing rule
    /// restricts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<FactionId>,
    /// D-UM-54's diversity cap, per mille of the House's titled hulls; absent,
    /// `[tacsim.procurement]`'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_permille: Option<u16>,
    /// The listing forms it may buy, distinct and non-empty; default all three.
    #[serde(default = "ProcurementRole::all")]
    pub roles: Vec<ProcurementRole>,
}
impl ProcurementPolicy {
    /// The policy's own bounds (decode): the agent neither #0 nor GOD; a named budget 1 to
    /// `MAX_PROCUREMENT_BUDGET_MINOR`, a named cap 1 to 1000; the roles non-empty and distinct.
    #[must_use]
    pub fn holds(&self) -> bool {
        let mut roles = self.roles.clone();
        roles.sort_unstable();
        roles.dedup();
        self.agent != ObjId(0)
            && self.agent != GOD
            && self
                .budget_minor_per_ic_month
                .is_none_or(|b| (1..=crate::config::MAX_PROCUREMENT_BUDGET_MINOR).contains(&b))
            && self.cap_permille.is_none_or(|c| (1..=1000).contains(&c))
            && !self.roles.is_empty()
            && roles.len() == self.roles.len()
    }
    /// The monthly budget, the policy's own or the configured default.
    #[must_use]
    pub fn budget(&self, defaults: &crate::config::ProcurementConfig) -> i64 {
        self.budget_minor_per_ic_month
            .unwrap_or(defaults.budget_minor_per_ic_month)
    }
    /// The diversity cap, the policy's own or the configured default.
    #[must_use]
    pub fn cap(&self, defaults: &crate::config::ProcurementConfig) -> u16 {
        self.cap_permille.unwrap_or(defaults.cap_permille)
    }
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
    /// Doc 17h §3 M8h (A1): each House's attesting character, by House: its procurement
    /// policy's agent where it has one (the ruling), else its own `attestation_agent`. A House
    /// with neither is left out (its filings are not attested).
    #[must_use]
    pub fn house_attestation_agents(&self) -> Vec<(OrgId, ObjId)> {
        [
            &self.houses.meridiani,
            &self.houses.coprates,
            &self.houses.tycho,
        ]
        .into_iter()
        .filter_map(|house| {
            self.procurement
                .iter()
                .find(|policy| policy.org == house.organization)
                .map(|policy| policy.agent)
                .or(house.attestation_agent)
                .map(|agent| (house.organization, agent))
        })
        .collect()
    }
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
        if !self.procurement_holds(&houses) {
            return Err(ModError::msg("launch deployment procurement differs"));
        }
        // Doc 17h §3 M8h (A1): a House names at most one attesting character: its procurement
        // policy's agent, or its own `attestation_agent`, never two different ones.
        for house in houses {
            if let Some(named) = house.attestation_agent
                && self
                    .procurement
                    .iter()
                    .any(|policy| policy.org == house.organization && policy.agent != named)
            {
                return Err(ModError::msg("launch deployment House attestation differs"));
            }
        }
        Ok(())
    }
    /// Doc 17h §3 M8f (P2): the procurement policies' shape. None without the player market
    /// (it buys from the market's listings); each policy holds its own bounds
    /// (`ProcurementPolicy::holds`), names one of the three named Houses' organisations, and no
    /// two name one organisation or one agent; the agent is not the market's clerk or a
    /// Registry operator.
    fn procurement_holds(&self, houses: &[&HouseDeployment; 3]) -> bool {
        if self.procurement.is_empty() {
            return true;
        }
        let Some(market) = &self.market else {
            return false;
        };
        self.procurement.len() <= houses.len()
            && self.procurement.iter().enumerate().all(|(index, policy)| {
                policy.holds()
                    && houses.iter().any(|house| house.organization == policy.org)
                    && policy.agent != market.clerk
                    && !market.registry_authority.operators.contains(&policy.agent)
                    && self
                        .procurement
                        .iter()
                        .take(index)
                        .all(|other| other.org != policy.org && other.agent != policy.agent)
            })
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
