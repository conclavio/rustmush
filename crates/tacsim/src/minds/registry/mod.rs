//! Private Registry computation. Frozen inputs and measured output do not grant
//! commercial authority; only acknowledged publication through Roll's closed
//! registered producer can create an effective certificate.

pub(crate) mod composition;
pub(crate) mod conduct;
mod evidence;
pub(crate) mod factory;
mod ford;
#[cfg(test)]
pub(in crate::minds) mod integration_support;
mod kernels;
mod machine;
mod manifest;
mod meter;
mod order_loss;
mod plan;
mod probes;
mod profiles;
mod registration;
mod runtime;
mod subject;
#[cfg(test)]
mod tests;
mod trials;

pub(crate) fn configure_roll(module: mush_roll::RollModule) -> mush_roll::RollModule {
    registration::roll_owners(module)
}

use mush_core::DefinitionDigest;
use mush_roll::commerce::rating::RatingError;
use serde::Serialize;

fn identity<T: Serialize>(
    domain: &str,
    value: &T,
    cap: usize,
) -> Result<DefinitionDigest, RatingError> {
    let value = serde_json::to_value(value).map_err(|_| RatingError::Invalid)?;
    DefinitionDigest::for_canonical_value_with_limit(domain, &value, cap)
        .map_err(|_| RatingError::Bound)
}
