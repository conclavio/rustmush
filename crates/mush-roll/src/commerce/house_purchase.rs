//! Closed original-house personal paid issuance. Explicit paired composition
//! remains mandatory; these structural records confer no payment or use rights.
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
}

mod admission_capacity;
pub(crate) mod roots;
pub mod writer;

pub(crate) mod audit;
pub(crate) mod history;

pub mod customer;
