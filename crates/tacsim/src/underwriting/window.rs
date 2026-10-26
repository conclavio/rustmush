use super::{CoverageError, CoverageTermsV1};
use mush_core::{ClockRef, ClockStamp};

/// A temporal calculation, not a policy, live clock observation or loss proof.
/// It retains the exact WorldIc generation, including through delayed claims.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageWindow {
    start: ClockStamp,
    end: ClockStamp,
    claim_delay_seconds: i64,
}
impl CoverageWindow {
    pub fn new(start: ClockStamp, terms: &CoverageTermsV1) -> Result<Self, CoverageError> {
        world_ic(&start)?;
        let end = ClockStamp {
            clock: start.clock.clone(),
            at: start
                .at
                .checked_add(terms.term_seconds())
                .ok_or(CoverageError::Arithmetic)?,
        };
        Ok(Self {
            start,
            end,
            claim_delay_seconds: terms.claim_delay_seconds(),
        })
    }
    pub fn start(&self) -> &ClockStamp {
        &self.start
    }
    pub fn end(&self) -> &ClockStamp {
        &self.end
    }

    /// Start is inclusive; end is exclusive. A clock rollback cannot turn a
    /// before-start loss into a covered one, and a generation change refuses.
    pub fn loss_payment_due(&self, loss_at: &ClockStamp) -> Result<ClockStamp, CoverageError> {
        world_ic(loss_at)?;
        if loss_at.clock != self.start.clock {
            return Err(CoverageError::Clock);
        }
        if loss_at.at < self.start.at || loss_at.at >= self.end.at {
            return Err(CoverageError::OutsideTerm);
        }
        Ok(ClockStamp {
            clock: loss_at.clock.clone(),
            at: loss_at
                .at
                .checked_add(self.claim_delay_seconds)
                .ok_or(CoverageError::Arithmetic)?,
        })
    }

    /// A covered loss may mature after policy expiry. Expiry must not release a
    /// liability that the real claim owner has already retained for that loss.
    /// This method neither recognizes a loss nor decides exclusions or payment.
    pub fn delay_elapsed(
        &self,
        loss_at: &ClockStamp,
        now: &ClockStamp,
    ) -> Result<bool, CoverageError> {
        let due = self.loss_payment_due(loss_at)?;
        world_ic(now)?;
        if now.clock != due.clock {
            return Err(CoverageError::Clock);
        }
        Ok(now.at >= due.at)
    }
}

fn world_ic(stamp: &ClockStamp) -> Result<(), CoverageError> {
    if matches!(stamp.clock, ClockRef::WorldIc { generation } if generation != 0) {
        Ok(())
    } else {
        Err(CoverageError::Clock)
    }
}
