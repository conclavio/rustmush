//! The fence's two document families (E6b, B9): `sr:fence:{id}` and
//! `sr:quote:{id}`.
//!
//! Both follow the E5a fold rule and `record.rs`'s own pattern exactly — the
//! persisted shape is a separate `…Wire` struct, and the live record is reached only
//! through a `TryFrom` that re-runs the constructor invariants with documented caps.
//! A document that fails it is skipped loudly at boot and the world boots: a quote is
//! a *projection* of a price anyone can ask for again, and a fence record is
//! re-registrable by staff, so losing one costs a listing and never an escrow.

use mush_core::ObjId;
use mush_economy::{AccountId, Amount, ContractId, EconomicDeadline, VaultId};
use mush_org::OrgId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::economy::config::{
    FENCE_DELAY_MAX_SECS, FENCE_DELAY_MIN_SECS, MAX_CATALOG_CODE_BYTES, MAX_PRICE_FACTOR_BYTES,
    MAX_PRICE_FACTORS, MAX_RUN_NAME_BYTES,
};
use crate::economy::record::RecordError;

/// A registered fence's id.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash,
)]
pub struct FenceId(pub u64);

impl std::fmt::Display for FenceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A fence quote's id.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash,
)]
pub struct QuoteId(pub u64);

impl std::fmt::Display for QuoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// `sr:fence:{id}` — a **staff-registered vendor organization** (E6b, M15).
///
/// It holds no money and no authority of its own: the account and the vault belong
/// to the organization, and all this record adds is that the world knows the org buys
/// goods and on what declared terms. `demand` is the one price lever a staffer sets,
/// in permille; moving it bumps `revision`, which every outstanding quote is bound
/// to — so a re-priced fence refuses its old quotes **deterministically** rather than
/// honouring or silently re-pricing them (D-E6-13, scenario 29).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SrFenceWire")]
pub struct SrFence {
    /// Identity (never reused).
    pub id: FenceId,
    /// The organization that is the fence.
    pub org: OrgId,
    /// Its custody vault — where a delivered object lands.
    pub vault: VaultId,
    /// Its own `sr.nuyen` account, derived from the org at registration.
    pub account: AccountId,
    /// The fence's shop sign. The organization's own name is rendered through the
    /// economy's hidden-org renderer and never from here.
    pub name: SmolStr,
    /// The declared demand adjustment, in permille, added to the settlement rate.
    pub demand: i64,
    /// Bumped by every write.
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct SrFenceWire {
    id: u64,
    org: OrgId,
    vault: VaultId,
    account: AccountId,
    #[serde(default)]
    name: SmolStr,
    #[serde(default)]
    demand: i64,
    #[serde(default)]
    revision: u64,
}

impl TryFrom<SrFenceWire> for SrFence {
    type Error = RecordError;

    fn try_from(w: SrFenceWire) -> Result<SrFence, RecordError> {
        if w.name.len() > MAX_RUN_NAME_BYTES {
            return Err(RecordError("a fence's name is too long"));
        }
        // The demand lever is bounded on both sides: it can never make a fence pay
        // more than the whole baseline, and the price clamp below zero is what stops
        // it minting a quote of nothing.
        if !(-1_000..=1_000).contains(&w.demand) {
            return Err(RecordError("a fence's demand adjustment is out of range"));
        }
        Ok(SrFence {
            id: FenceId(w.id),
            org: w.org,
            vault: w.vault,
            account: w.account,
            name: w.name,
            demand: w.demand,
            revision: w.revision,
        })
    }
}

/// How a fence deal settles (answer 15).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FenceSettlement {
    /// Immediate, for less: it settles on the deposit alone.
    Immediate,
    /// Consignment, for more: the deposit **plus** a delay, shown in the offer.
    Consign {
        /// The delay in seconds, clamped to 24–72 h at quote time.
        delay: i64,
    },
}

/// One declared input to a quote's price (D-E6-11).
///
/// Every factor is **observable**: the baseline is source-controlled, the settlement
/// rate is configured, and the demand adjustment is on the fence's own public record.
/// A hidden fact contributes no factor at all — not even a discount — so the basis a
/// seller reads is the whole of the arithmetic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceFactor {
    /// What it is.
    pub label: SmolStr,
    /// Its contribution, in permille.
    pub permille: i64,
}

/// The state a quote was priced against (D-E6-13, scenario 29).
///
/// `RevisionSet` is a `mush-core` type and not an economy one, so this is the pair of
/// stamps the economy actually has — and (E6b verify fold, D1) **both halves are
/// declared**. `EconomicAsset.revision` was the obvious first choice and was an
/// oracle: every asset op bumps it, including a **covert** `ReportStolen`, so filing
/// one flipped the holder's outstanding quote to `lapsed` where they could read it.
/// The asset half is therefore a hash of the asset's own `CatalogRef.code` — what
/// the quote was priced from, and nothing a hidden fact can move.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteObserved {
    /// A stable hash of the declared catalogue code the quote was priced against.
    pub declared: u64,
    /// The fence record's revision — its declared terms.
    pub fence: u64,
}

/// `sr:quote:{id}` — one deterministic fence offer (E6b, answers 13 and 15).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FenceQuoteWire")]
pub struct FenceQuote {
    /// Identity (never reused).
    pub id: QuoteId,
    /// Which fence made it.
    pub fence: FenceId,
    /// Who it was made to.
    pub seller: ObjId,
    /// The object it is about. Title never moves (D-E6-10).
    pub object: ObjId,
    /// The catalog code it was priced against.
    pub catalog: SmolStr,
    /// What the fence will pay.
    pub offer: Amount,
    /// The whole of the arithmetic, in declared factors.
    pub basis: Vec<PriceFactor>,
    /// Immediate or consignment.
    pub settlement: FenceSettlement,
    /// When it lapses, on `RealUtc` — its clock is declared (answer 11).
    pub expires: EconomicDeadline,
    /// The revisions it was priced against.
    pub observed: QuoteObserved,
    /// The contract, once `+fence/accept` drafted one.
    #[serde(default)]
    pub contract: Option<ContractId>,
    /// Bumped by every write.
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct FenceQuoteWire {
    id: u64,
    fence: u64,
    seller: ObjId,
    object: ObjId,
    #[serde(default)]
    catalog: SmolStr,
    #[serde(default = "zero")]
    offer: Amount,
    #[serde(default)]
    basis: Vec<PriceFactor>,
    settlement: FenceSettlement,
    expires: EconomicDeadline,
    observed: QuoteObserved,
    #[serde(default)]
    contract: Option<u64>,
    #[serde(default)]
    revision: u64,
}

fn zero() -> Amount {
    Amount::ZERO
}

impl TryFrom<FenceQuoteWire> for FenceQuote {
    type Error = RecordError;

    fn try_from(w: FenceQuoteWire) -> Result<FenceQuote, RecordError> {
        if w.catalog.trim().is_empty() || w.catalog.len() > MAX_CATALOG_CODE_BYTES {
            return Err(RecordError("a quote names no usable catalogue code"));
        }
        if w.offer.minor() <= 0 {
            return Err(RecordError("a quote offers nothing"));
        }
        if w.basis.len() > MAX_PRICE_FACTORS
            || w.basis
                .iter()
                .any(|f| f.label.trim().is_empty() || f.label.len() > MAX_PRICE_FACTOR_BYTES)
        {
            return Err(RecordError("a quote's price basis is malformed"));
        }
        if let FenceSettlement::Consign { delay } = w.settlement
            && !(FENCE_DELAY_MIN_SECS..=FENCE_DELAY_MAX_SECS).contains(&delay)
        {
            return Err(RecordError("a consignment's delay is outside 24-72 hours"));
        }
        Ok(FenceQuote {
            id: QuoteId(w.id),
            fence: FenceId(w.fence),
            seller: w.seller,
            object: w.object,
            catalog: w.catalog,
            offer: w.offer,
            basis: w.basis,
            settlement: w.settlement,
            expires: w.expires,
            observed: w.observed,
            contract: w.contract.map(ContractId),
            revision: w.revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_with_no_catalogue_code_never_loads() {
        let bad = serde_json::json!({
            "id": 1, "fence": 1, "seller": 5, "object": 9,
            "offer": 100, "settlement": "immediate",
            "expires": { "clock": "RealUtc", "at": 10 },
            "observed": { "declared": 1, "fence": 1 },
        });
        assert!(serde_json::from_value::<FenceQuote>(bad).is_err());
    }

    #[test]
    fn a_consignment_outside_the_declared_window_never_loads() {
        let bad = serde_json::json!({
            "id": 1, "fence": 1, "seller": 5, "object": 9, "catalog": "ares.predator",
            "offer": 100, "settlement": { "consign": { "delay": 60 } },
            "expires": { "clock": "RealUtc", "at": 10 },
            "observed": { "declared": 1, "fence": 1 },
        });
        assert!(serde_json::from_value::<FenceQuote>(bad).is_err());
    }

    #[test]
    fn a_fence_demand_outside_permille_never_loads() {
        let bad = serde_json::json!({
            "id": 1, "org": 1, "vault": 1, "account": 1, "demand": 5_000,
        });
        assert!(serde_json::from_value::<SrFence>(bad).is_err());
    }
}
