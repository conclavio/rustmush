//! Only the paired Roll writer constructs renewal retention images. The
//! operation and next-term roots are real ordinary Roll records before money.
//!
//! Doc 46 CT.5: written once in `renewal_twin::roots`; these are its types at the
//! House issuer.
use super::super::renewal_twin::House;

pub(crate) type RenewalFence = super::super::renewal_twin::roots::RenewalFence<House>;
#[cfg(test)]
pub(crate) use super::super::renewal_twin::roots::Publication;
pub(crate) type RenewalRoots = super::super::renewal_twin::roots::RenewalRoots<House>;
