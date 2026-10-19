//! Exact bounded typed rows. Their decoders establish structure only; provider
//! fences and the complete native/Root/host audit establish actual provenance.
use super::{CoverageError, policy::*, records::*};
use mush_core::mechanics::CanonicalId;
use mush_core::{
    ActionTxn, CommitToken, ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle,
    StateFamilyId,
};
use serde::{Deserialize, Serialize};

pub(super) const MAX_ROWS: u64 = 65536;
pub(super) trait Row:
    Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static
{
    const FAMILY: &'static str;
    const LIMIT: usize;
    fn key(&self) -> String;
    fn decode(bytes: &[u8]) -> Result<Self, CoverageError>;
    fn key_valid(key: &str) -> bool {
        CommitToken::from_hex(key).is_ok_and(|v| v.to_hex() == key)
    }
    fn validate(&self) -> Result<(), CoverageError> {
        let bytes =
            mush_core::canonical_json_bytes_with_limit(&value(self, Self::LIMIT)?, Self::LIMIT)
                .map_err(|_| CoverageError::Bound)?;
        Self::decode(&bytes).map(|_| ())
    }
}
macro_rules! row {
    ($ty:ty, $family:literal, $limit:expr, $field:ident) => {
        impl Row for $ty {
            const FAMILY: &'static str = $family;
            const LIMIT: usize = $limit;
            fn key(&self) -> String {
                self.$field.to_hex()
            }
            fn decode(bytes: &[u8]) -> Result<Self, CoverageError> {
                <$ty>::decode(bytes)
            }
        }
    };
}
row!(
    Publication,
    "tacsim_coverage_products",
    MAX_PUBLICATION_BYTES,
    operation
);
row!(Quote, "tacsim_coverage_quotes", MAX_QUOTE_BYTES, operation);
row!(Policy, "tacsim_coverage_policies", POLICY_BYTES, operation);
row!(
    Exposure,
    "tacsim_coverage_exposures",
    EXPOSURE_BYTES,
    policy
);
row!(
    Terminal,
    "tacsim_coverage_terminals",
    TERMINAL_BYTES,
    policy
);
row!(Receipt, "tacsim_coverage_receipts", RECEIPT_BYTES, policy);
impl Row for HullHead {
    const FAMILY: &'static str = "tacsim_coverage_hulls";
    const LIMIT: usize = HULL_BYTES;
    fn key(&self) -> String {
        format!("{:08x}", self.hull.0)
    }
    fn decode(bytes: &[u8]) -> Result<Self, CoverageError> {
        Self::decode(bytes)
    }
    fn key_valid(key: &str) -> bool {
        key.len() == 8
            && u32::from_str_radix(key, 16).is_ok_and(|id| id != 0 && format!("{id:08x}") == key)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Counts {
    format: u16,
    pub(super) products: u64,
    pub(super) quotes: u64,
    pub(super) policies: u64,
}
impl Row for Counts {
    const FAMILY: &'static str = "tacsim_coverage_counts";
    const LIMIT: usize = 1024;
    fn key(&self) -> String {
        "registry".into()
    }
    fn key_valid(key: &str) -> bool {
        key == "registry"
    }
    fn decode(bytes: &[u8]) -> Result<Self, CoverageError> {
        let v: Self = super::records::decode(bytes, Self::LIMIT)?;
        if v.format != 1
            || [v.products, v.quotes, v.policies]
                .iter()
                .any(|n| *n > MAX_ROWS)
        {
            return Err(CoverageError::Bound);
        }
        Ok(v)
    }
}
#[derive(Clone)]
pub(crate) struct Families {
    pub(super) products: StateFamilyHandle<String, Publication>,
    pub(super) quotes: StateFamilyHandle<String, Quote>,
    pub(super) policies: StateFamilyHandle<String, Policy>,
    pub(super) exposures: StateFamilyHandle<String, Exposure>,
    pub(super) hulls: StateFamilyHandle<String, HullHead>,
    pub(super) terminals: StateFamilyHandle<String, Terminal>,
    pub(super) receipts: StateFamilyHandle<String, Receipt>,
    pub(super) counts: StateFamilyHandle<String, Counts>,
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(super) fn descriptor<R: Row>() -> Result<StateFamilyDescriptor, ModError> {
    let id = |suffix| CanonicalId::new(format!("{}.{}.v1", R::FAMILY, suffix)).map_err(bad);
    StateFamilyDescriptor::new(
        StateFamilyId::new(R::FAMILY).map_err(bad)?,
        id("key")?,
        id("value")?,
        1,
        id("decode")?,
        id("validate")?,
        id("canonical")?,
    )
    .and_then(|d| d.with_limits(32, R::LIMIT))
    .map_err(bad)
}
fn register<R: Row>(reg: &mut Registrar<'_>) -> Result<StateFamilyHandle<String, R>, ModError> {
    reg.state_family(
        descriptor::<R>()?,
        |key: &String| {
            if R::key_valid(key) {
                Ok(key.clone())
            } else {
                Err("coverage key".into())
            }
        },
        |value: &serde_json::Value| {
            let bytes = mush_core::canonical_json_bytes_with_limit(value, R::LIMIT)
                .map_err(|e| e.to_string())?;
            R::decode(&bytes).map_err(|e| e.to_string())
        },
        |row: &R| row.validate().map_err(|e| e.to_string()),
        |row: &R| serde_json::to_value(row).map_err(|e| e.to_string()),
    )
}
impl Families {
    pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        if reg.module() != crate::NS {
            return Err(bad("coverage families are Tacsim-owned"));
        }
        Ok(Self {
            products: register(reg)?,
            quotes: register(reg)?,
            policies: register(reg)?,
            exposures: register(reg)?,
            hulls: register(reg)?,
            terminals: register(reg)?,
            receipts: register(reg)?,
            counts: register(reg)?,
        })
    }
    pub(super) fn admit(&self, txn: &mut ActionTxn<'_>, kind: usize) -> Result<(), CoverageError> {
        let key = "registry".to_owned();
        let before = txn
            .get(&self.counts, &key)
            .map_err(|_| CoverageError::Authority)?
            .cloned();
        let mut after = before.clone().unwrap_or(Counts {
            format: 1,
            products: 0,
            quotes: 0,
            policies: 0,
        });
        after.validate()?;
        let count = match kind {
            0 => &mut after.products,
            1 => &mut after.quotes,
            2 => &mut after.policies,
            _ => return Err(CoverageError::Product),
        };
        *count = count.checked_add(1).ok_or(CoverageError::Bound)?;
        after.validate()?;
        if before.is_some() {
            txn.replace(&self.counts, &key, after.clone())
        } else {
            txn.insert(&self.counts, &key, after.clone())
        }
        .map_err(|_| CoverageError::Authority)?;
        if txn
            .get(&self.counts, &key)
            .map_err(|_| CoverageError::Authority)?
            != Some(&after)
        {
            return Err(CoverageError::Product);
        }
        Ok(())
    }
}
pub(super) fn read<R: Row>(
    txn: &mut ActionTxn<'_>,
    f: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<R, CoverageError> {
    let row = txn
        .get(f, &key)
        .map_err(|_| CoverageError::Authority)?
        .cloned()
        .ok_or(CoverageError::Product)?;
    row.validate()?;
    if row.key() != key {
        return Err(CoverageError::Product);
    }
    Ok(row)
}
pub(super) fn insert<R: Row + PartialEq>(
    txn: &mut ActionTxn<'_>,
    f: &StateFamilyHandle<String, R>,
    row: &R,
) -> Result<(), CoverageError> {
    row.validate()?;
    let key = row.key();
    txn.insert(f, &key, row.clone())
        .map_err(|_| CoverageError::Authority)?;
    if txn.get(f, &key).map_err(|_| CoverageError::Authority)? != Some(row) {
        return Err(CoverageError::Product);
    }
    Ok(())
}
pub(super) fn replace<R: Row + PartialEq>(
    txn: &mut ActionTxn<'_>,
    f: &StateFamilyHandle<String, R>,
    row: &R,
) -> Result<(), CoverageError> {
    row.validate()?;
    let key = row.key();
    txn.replace(f, &key, row.clone())
        .map_err(|_| CoverageError::Authority)?;
    if txn.get(f, &key).map_err(|_| CoverageError::Authority)? != Some(row) {
        return Err(CoverageError::Product);
    }
    Ok(())
}
