//! Optional trusted refusal-only product admission. No receipt, owner state,
//! action grant or mutable world is exposed to the policy implementation.
use crate::commerce::{house_catalogue::HouseIssuer, records::PublicationError};
use mush_core::{ObjId, World};

pub trait HouseBuyerAdmission: Send + Sync + std::fmt::Debug {
    /// Called for an actual proved issuer and authenticated personal buyer.
    /// A successful check only permits the existing owner proofs to continue.
    fn check(
        &self,
        world: &World,
        issuer: HouseIssuer,
        buyer: ObjId,
    ) -> Result<(), PublicationError>;
}
