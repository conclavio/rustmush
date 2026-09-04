//! Checked bridge from the action kernel's recorded RNG to existing dice
//! evaluators.

use mush_core::{CanonicalId, ResolutionRng, ResolutionRngError};
use rand::RngCore;

use crate::{RollCtx, RollLimits};

struct ResolutionDiceRng<'a> {
    source: &'a mut ResolutionRng,
    label: &'a CanonicalId,
    failure: Option<ResolutionRngError>,
}

impl ResolutionDiceRng<'_> {
    fn finish(self) -> Result<(), ResolutionRngError> {
        self.failure.map_or(Ok(()), Err)
    }

    fn checked_u32(&mut self) -> u32 {
        if self.failure.is_some() {
            return 0;
        }
        match self.source.draw_u32(self.label) {
            Ok(value) => value,
            Err(error) => {
                self.failure = Some(error);
                0
            }
        }
    }
}

impl RngCore for ResolutionDiceRng<'_> {
    fn next_u32(&mut self) -> u32 {
        self.checked_u32()
    }

    fn next_u64(&mut self) -> u64 {
        (u64::from(self.checked_u32()) << 32) | u64::from(self.checked_u32())
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(4) {
            let bytes = self.checked_u32().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}

/// Run a pure dice calculation on one independent, recorded resolution
/// stream. If the kernel draw budget is exhausted, the provisional result is
/// discarded and the action must fail before authoritative mutation.
pub fn with_resolution_stream<T>(
    source: &mut ResolutionRng,
    label: &CanonicalId,
    limits: RollLimits,
    resolve: impl FnOnce(&mut RollCtx<'_>) -> T,
) -> Result<T, ResolutionRngError> {
    let mut adapter = ResolutionDiceRng {
        source,
        label,
        failure: None,
    };
    let result = {
        let mut context = RollCtx::with_limits(&mut adapter, limits);
        resolve(&mut context)
    };
    adapter.finish()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{
        CanonicalHash, GOD, IdempotencyKey, IdempotencyScope, MAX_RESOLUTION_DRAWS, OwnerId,
        PrincipalId, RequestUid, ResolutionKeyRing, ResolutionSecret, ScopedIdempotencyKey,
    };
    use serde_json::json;

    fn source() -> ResolutionRng {
        let request = RequestUid::from_bytes([0x61; 16]);
        let principal = PrincipalId::Object { id: GOD };
        let key = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("toy").unwrap(),
                namespace: CanonicalId::new("dice").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        );
        let inputs = CanonicalHash::for_canonical_value(
            "dice_test_inputs.v1",
            1,
            &json!({ "expression": "3d6" }),
            1024,
        )
        .unwrap();
        ResolutionKeyRing::new(ResolutionSecret::new(1, [0x72; 32]).unwrap(), [])
            .unwrap()
            .start(&key, &CanonicalId::new("roll").unwrap(), inputs)
            .unwrap()
    }

    #[test]
    fn existing_expression_evaluator_records_every_die() {
        let expression = crate::parse("3d6").unwrap();
        let label = CanonicalId::new("primary_pool").unwrap();
        let mut first = source();
        let outcome =
            with_resolution_stream(&mut first, &label, RollLimits::default(), |context| {
                expression.eval(context)
            })
            .unwrap();
        assert_eq!(first.draws().len(), 3);

        let mut repeated = source();
        let repeated_outcome =
            with_resolution_stream(&mut repeated, &label, RollLimits::default(), |context| {
                expression.eval(context)
            })
            .unwrap();
        assert_eq!(outcome, repeated_outcome);
        assert_eq!(first.draws(), repeated.draws());
    }

    #[test]
    fn exhausted_kernel_budget_discards_provisional_dice_result() {
        let label = CanonicalId::new("primary_pool").unwrap();
        let mut source = source();
        for _ in 0..MAX_RESOLUTION_DRAWS {
            source.draw_u32(&label).unwrap();
        }
        let expression = crate::parse("1d6").unwrap();
        let error = with_resolution_stream(&mut source, &label, RollLimits::default(), |context| {
            expression.eval(context)
        })
        .unwrap_err();
        assert_eq!(
            error,
            ResolutionRngError::DrawLimit {
                max: MAX_RESOLUTION_DRAWS
            }
        );
    }
}
