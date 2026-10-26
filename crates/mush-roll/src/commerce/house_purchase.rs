//! Closed original-house personal paid issuance. Explicit paired composition
//! remains mandatory; these structural records confer no payment or use rights.
mod admission;
pub use admission::HouseBuyerAdmission;
mod economics;
mod records;
pub use economics::{
    HouseDownstreamV1, HouseGrantEconomicsV1, HouseRenewalTemplateV1, HouseTermEconomicsV1,
};
pub use records::HouseAllocationV1;
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HouseProtocol {
    HouseV1,
    HouseV2,
}

pub(crate) mod admission_capacity;
pub(crate) mod roots;
pub mod writer;

pub(crate) mod audit;
pub(crate) mod history;

pub mod customer;

impl HouseProtocol {
    pub const fn version(self) -> u32 {
        match self {
            Self::HouseV1 => 1,
            Self::HouseV2 => 2,
        }
    }
    pub const fn format(self) -> u16 {
        match self {
            Self::HouseV1 => 1,
            Self::HouseV2 => 2,
        }
    }
    pub const fn seats(self) -> u16 {
        match self {
            Self::HouseV1 => 1,
            Self::HouseV2 => 2,
        }
    }
    pub const fn protocol_name(self) -> &'static str {
        match self {
            Self::HouseV1 => "house_mind_purchase_v1",
            Self::HouseV2 => "house_mind_purchase_v2",
        }
    }
    pub fn from_format(format: u16) -> Result<Self, super::records::PublicationError> {
        match format {
            1 => Ok(Self::HouseV1),
            2 => Ok(Self::HouseV2),
            _ => Err(super::records::PublicationError::Invalid),
        }
    }
}
