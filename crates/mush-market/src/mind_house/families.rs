//! Exact bounded Market state. Permanent listings preserve the publication
//! cause of immutable royalty chains; this cut never deletes that evidence.
use super::codec::Row;
use super::{HouseConfig, HouseListingV1, HousePurchaseV1, HouseQuoteV1, HouseReceiptV1};
use mush_core::mechanics::CanonicalId;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    ActionTxn, CommitToken, ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle,
    StateFamilyId,
};
use mush_roll::commerce::house_catalogue::HouseSaleReader;
use serde::{Deserialize, Serialize};

pub(super) const MAX_RETAINED: u64 = 32768;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Counts {
    format: u16,
    listings: u64,
    quotes: u64,
    operations: u64,
}
impl Counts {
    fn empty() -> Self {
        Self {
            format: 1,
            listings: 0,
            quotes: 0,
            operations: 0,
        }
    }
}
impl Row for Counts {
    const FAMILY: &'static str = "market_mind_house_counts";
    const MAX_BYTES: usize = 1024;
    fn key(&self) -> String {
        "registry".into()
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.listings > MAX_RETAINED
            || self.quotes > MAX_RETAINED
            || self.operations > MAX_RETAINED
        {
            return Err("entitlement count bound".into());
        }
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct Families {
    pub(super) listings: StateFamilyHandle<CommitToken, HouseListingV1>,
    pub(super) quotes: StateFamilyHandle<CommitToken, HouseQuoteV1>,
    pub(super) operations: StateFamilyHandle<CommitToken, HousePurchaseV1>,
    pub(super) receipts: StateFamilyHandle<CommitToken, HouseReceiptV1>,
    pub(super) counts: StateFamilyHandle<String, Counts>,
    pub(super) reader: OwnerReadHandle<HouseSaleReader>,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor<R: Row>() -> Result<StateFamilyDescriptor, ModError> {
    let id = |suffix| CanonicalId::new(format!("{}.{}.v1", R::FAMILY, suffix)).map_err(error);
    StateFamilyDescriptor::new(
        StateFamilyId::new(R::FAMILY).map_err(error)?,
        id("key")?,
        id("value")?,
        1,
        id("decode")?,
        id("validate")?,
        id("canonical")?,
    )
    .and_then(|d| d.with_limits(32, R::MAX_BYTES))
    .map_err(error)
}
fn decode<R: Row>(value: &serde_json::Value) -> Result<R, String> {
    let bytes = mush_core::canonical_json_bytes_with_limit(value, R::MAX_BYTES)
        .map_err(|e| e.to_string())?;
    super::codec::decode(&bytes)
}
fn family<R: Row + Clone + Send + Sync + 'static>(
    reg: &mut Registrar<'_>,
) -> Result<StateFamilyHandle<CommitToken, R>, ModError> {
    reg.state_family(
        descriptor::<R>()?,
        |key: &CommitToken| Ok(key.to_hex()),
        decode::<R>,
        R::validate,
        |r: &R| serde_json::to_value(r).map_err(|e| e.to_string()),
    )
}
impl Families {
    pub(super) fn register(
        reg: &mut Registrar<'_>,
        config: &HouseConfig,
    ) -> Result<Self, ModError> {
        Ok(Self {
            listings: family(reg)?,
            quotes: family(reg)?,
            operations: family(reg)?,
            receipts: family(reg)?,
            counts: reg.state_family(
                descriptor::<Counts>()?,
                |key: &String| {
                    if key == "registry" {
                        Ok(key.clone())
                    } else {
                        Err("unknown entitlement counter".into())
                    }
                },
                decode::<Counts>,
                Counts::validate,
                |r: &Counts| serde_json::to_value(r).map_err(|e| e.to_string()),
            )?,
            reader: HouseSaleReader::request(reg, &config.manifest)?,
        })
    }
    pub(super) fn add_listing(
        &self,
        txn: &mut ActionTxn<'_>,
        row: HouseListingV1,
    ) -> Result<(), String> {
        self.update_count(txn, |c| {
            c.listings = c.listings.checked_add(1).ok_or("listing count overflow")?;
            Ok(())
        })?;
        row.validate()?;
        txn.insert(&self.listings, &row.publication, row.clone())
            .map_err(|e| e.to_string())?;
        if txn
            .get(&self.listings, &row.publication)
            .map_err(|e| e.to_string())?
            != Some(&row)
        {
            return Err("listing post-image differs".into());
        }
        Ok(())
    }
    pub(super) fn add_quote(
        &self,
        txn: &mut ActionTxn<'_>,
        row: HouseQuoteV1,
    ) -> Result<(), String> {
        self.update_count(txn, |c| {
            c.quotes = c.quotes.checked_add(1).ok_or("quote count overflow")?;
            Ok(())
        })?;
        row.validate()?;
        txn.insert(&self.quotes, &row.publication, row.clone())
            .map_err(|e| e.to_string())?;
        if txn
            .get(&self.quotes, &row.publication)
            .map_err(|e| e.to_string())?
            != Some(&row)
        {
            return Err("quote post-image differs".into());
        }
        Ok(())
    }
    pub(super) fn consume_quote(
        &self,
        txn: &mut ActionTxn<'_>,
        quote: CommitToken,
    ) -> Result<(), String> {
        self.update_count(txn, |c| {
            c.quotes = c.quotes.checked_sub(1).ok_or("quote count underflow")?;
            c.operations = c
                .operations
                .checked_add(1)
                .ok_or("operation count overflow")?;
            Ok(())
        })?;
        txn.remove(&self.quotes, &quote)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn update_count(
        &self,
        txn: &mut ActionTxn<'_>,
        change: impl FnOnce(&mut Counts) -> Result<(), String>,
    ) -> Result<(), String> {
        let key = "registry".to_owned();
        let before = txn
            .get(&self.counts, &key)
            .map_err(|e| e.to_string())?
            .cloned();
        let mut after = before.clone().unwrap_or_else(Counts::empty);
        after.validate()?;
        change(&mut after)?;
        after.validate()?;
        if before.is_some() {
            txn.replace(&self.counts, &key, after.clone())
        } else {
            txn.insert(&self.counts, &key, after.clone())
        }
        .map_err(|e| e.to_string())?;
        if txn.get(&self.counts, &key).map_err(|e| e.to_string())? != Some(&after) {
            return Err("entitlement counter post-image differs".into());
        }
        Ok(())
    }
    pub(super) fn check_counts(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        listings: u64,
        quotes: u64,
        operations: u64,
    ) -> Result<(), String> {
        let row = ctx
            .action_state(&self.counts, &"registry".into())
            .map_err(|e| e.to_string())?;
        if let Some(row) = row {
            row.validate()?;
            if (row.listings, row.quotes, row.operations) != (listings, quotes, operations) {
                return Err("entitlement counters differ from actual rows".into());
            }
        } else if listings != 0 || quotes != 0 || operations != 0 {
            return Err("entitlement counters missing".into());
        }
        Ok(())
    }
}
