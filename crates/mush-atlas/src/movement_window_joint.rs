//! Read-only join of real Atlas selection, a Core-minted preview and Sim's
//! ordered native capture. Publication still requires all actual owner binds.

use super::*;
use mush_core::locomotion::preview::LocomotionPreview;
use mush_sim::window_movement::{OrderedWindowMovementCapture, WindowTarget, capture_ordered};

/// There is no decoder, receipt, owner patch, installation or eligibility
/// capability here. The projection is proposal data for the separate kernel.
pub struct JointWindowMovementCapture {
    atlas: WindowArrivalPlan,
    sim: OrderedWindowMovementCapture,
    projection: WindowProjection,
}

impl WindowArrivalPlan {
    /// Join the exact real Core route and selected destination. Bounds and
    /// unsupported native-owner cases are enforced by the actual Sim capture.
    /// The caller must mint its preview from `projection_target()` first.
    pub fn capture_sim(
        self,
        world: &World,
        core: LocomotionPreview,
    ) -> Result<JointWindowMovementCapture, CaptureError> {
        self.recheck(world)?;
        let (room, target) = self.projection_target()?;
        if core.actor() != self.capture.actor()
            || core.source() != self.capture.from()
            || core.admitted_room() != self.capture.to()
            || core.destination() != room
            || core.at_utc() != self.capture.at()
            || world.now() != self.capture.at()
        {
            return Err(CaptureError::Invalid);
        }
        let target =
            target.map(|t| WindowTarget::at(DomainId(t.domain()), t.x_cm(), t.y_cm(), t.z_cm()));
        let sim = capture_ordered(world, core, target).map_err(|_| CaptureError::Invalid)?;
        let projection = self.projection_proposal(sim.digest())?;
        Ok(JointWindowMovementCapture {
            atlas: self,
            sim,
            projection,
        })
    }
}

impl JointWindowMovementCapture {
    pub fn atlas(&self) -> &WindowArrivalPlan {
        &self.atlas
    }
    pub fn sim(&self) -> &OrderedWindowMovementCapture {
        &self.sim
    }
    pub fn projection(&self) -> &WindowProjection {
        &self.projection
    }
    /// Retain each owner's original evidence and UTC; never rerun callbacks.
    pub fn recheck(&self, world: &World) -> Result<(), CaptureError> {
        self.atlas.recheck(world)?;
        self.sim.recheck(world).map_err(|_| CaptureError::Invalid)
    }
    /// The real owner adapters must still attest, bind and record these parts.
    pub fn into_parts(
        self,
    ) -> (
        WindowArrivalPlan,
        OrderedWindowMovementCapture,
        WindowProjection,
    ) {
        (self.atlas, self.sim, self.projection)
    }
}
