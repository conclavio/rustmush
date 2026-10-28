use super::records::{MindPaidResaleOffer, MindPaidResaleOperation, MindPaidResaleReceipt};
use mush_core::mechanics::CanonicalId;
use mush_core::{
    CommitToken, ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use serde::{Serialize, de::DeserializeOwned};

pub(super) trait Row: Clone + Serialize + DeserializeOwned + Send + Sync + 'static {
    const FAMILY: &'static str;
    const MAX_BYTES: usize;
    fn validate(&self) -> Result<(), String>;
    fn key(&self) -> CommitToken;
}
macro_rules! row {
    ($t:ty, $key:ident) => {
        impl Row for $t {
            const FAMILY: &'static str = <$t>::FAMILY;
            const MAX_BYTES: usize = <$t>::MAX_BYTES;
            fn validate(&self) -> Result<(), String> {
                <$t>::validate(self)
            }
            fn key(&self) -> CommitToken {
                self.$key
            }
        }
    };
}
row!(MindPaidResaleOffer, operation);
row!(MindPaidResaleOperation, operation);
// Fund reserves this exact key before the terminal commit is known.
row!(MindPaidResaleReceipt, acceptance);
#[derive(Clone)]
pub(crate) struct Families {
    pub(super) offers: StateFamilyHandle<CommitToken, MindPaidResaleOffer>,
    pub(super) operations: StateFamilyHandle<CommitToken, MindPaidResaleOperation>,
    pub(super) receipts: StateFamilyHandle<CommitToken, MindPaidResaleReceipt>,
    /// 17g C2f: the per-(buyer, licence) offer index (`super::index`); the
    /// market alone writes it, so it is not delegated to Roll.
    pub(super) buyer_offers: StateFamilyHandle<
        (mush_core::ObjId, mush_roll::commerce::LicenceId),
        super::index::ResaleBuyerOffers,
    >,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn family<R: Row>(reg: &mut Registrar<'_>) -> Result<StateFamilyHandle<CommitToken, R>, ModError> {
    let id = |suffix| CanonicalId::new(format!("{}.{suffix}.v1", R::FAMILY)).map_err(error);
    let d = StateFamilyDescriptor::new(
        StateFamilyId::new(R::FAMILY).map_err(error)?,
        id("key")?,
        id("value")?,
        1,
        id("decode")?,
        id("validate")?,
        id("canonical")?,
    )
    .and_then(|d| d.with_limits(32, R::MAX_BYTES))
    .map_err(error)?;
    reg.state_family(
        d,
        |key: &CommitToken| Ok(key.to_hex()),
        |value: &serde_json::Value| {
            let bytes = mush_core::canonical_json_bytes_with_limit(value, R::MAX_BYTES)
                .map_err(|e| e.to_string())?;
            let row: R = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            row.validate()?;
            if serde_json::to_value(&row).map_err(|e| e.to_string())? != *value {
                return Err("noncanonical paid resale row".into());
            }
            Ok(row)
        },
        R::validate,
        |row: &R| serde_json::to_value(row).map_err(|e| e.to_string()),
    )
}
impl Families {
    pub(super) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        Ok(Self {
            offers: family(reg)?,
            operations: family(reg)?,
            receipts: family(reg)?,
            buyer_offers: super::index::register(reg)?,
        })
    }
    /// 17g C2g: the seller offers and the per-(buyer, licence) offer index,
    /// which `mind_commands`' `ResaleOffers` point-reads for the calling
    /// actor only (read only; every write stays in this module).
    pub(crate) const fn offers(&self) -> &StateFamilyHandle<CommitToken, MindPaidResaleOffer> {
        &self.offers
    }
    pub(crate) const fn buyer_offers(
        &self,
    ) -> &StateFamilyHandle<
        (mush_core::ObjId, mush_roll::commerce::LicenceId),
        super::index::ResaleBuyerOffers,
    > {
        &self.buyer_offers
    }
}
pub(super) fn envelope<R: Row>(
    family: &StateFamilyHandle<CommitToken, R>,
    key: CommitToken,
    value: serde_json::Value,
) -> Result<(), String> {
    let d = family.descriptor();
    let doc = serde_json::json!({"family":{"owner":family.owner(),"id":d.id()},"key":key.to_hex(),"schema":{"id":d.value_schema(),"version":d.schema_version()},"value":value});
    mush_core::canonical_json_bytes_with_limit(&doc, d.max_record_bytes())
        .map(|_| ())
        .map_err(|_| "complete paid resale document exceeds bound".into())
}
