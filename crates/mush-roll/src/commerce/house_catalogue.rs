//! Current organization issuance facts, separate from a person's free owner
//! licence. Publication requires scoped curator authority; an acknowledged
//! listing belongs to the organization and outlives that curator's departure.
mod authority;
mod reader;
pub use authority::HouseIssuer;
pub use reader::{HouseSaleFacts, HouseSaleReader, HouseSaleRequest};
pub(crate) use reader::{descriptor, register};
