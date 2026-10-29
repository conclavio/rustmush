//! Separate v2 primary capability and immutable proposed window header. No
//! caller can mint its captured route, preparation binding, or Core read seed.

use std::collections::BTreeSet;

use crate::locomotion::preview::LocomotionPreview;
use crate::locomotion::transition::{LocomotionDraft, LocomotionIntent};
use crate::participant::{OwnerPrepareContext, ParticipantError};
use crate::{
    ActionInvocation, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World,
};
use serde::{Deserialize, Serialize};

use super::{exact_window_action, refused};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowMetricTarget {
    domain: u64,
    x_cm: i64,
    y_cm: i64,
    z_cm: i32,
}
impl WindowMetricTarget {
    pub const fn at(domain: u64, x_cm: i64, y_cm: i64, z_cm: i32) -> Self {
        Self {
            domain,
            x_cm,
            y_cm,
            z_cm,
        }
    }
    pub const fn domain(&self) -> u64 {
        self.domain
    }
    pub const fn x_cm(&self) -> i64 {
        self.x_cm
    }
    pub const fn y_cm(&self) -> i64 {
        self.y_cm
    }
    pub const fn z_cm(&self) -> i32 {
        self.z_cm
    }
}
/// Shared proposal data, never a geometry/placement grant in isolation. Every
/// actual observer's output must bind this exact header through its intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowProjection {
    format: u16,
    projected_room: ObjId,
    atlas_capture_digest: DefinitionDigest,
    sim_window_capture_digest: DefinitionDigest,
    #[serde(deserialize_with = "required_target")]
    target: Option<WindowMetricTarget>,
}
fn required_target<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<WindowMetricTarget>, D::Error> {
    Option::deserialize(d)
}
impl WindowProjection {
    pub fn new(
        projected_room: ObjId,
        atlas_capture_digest: DefinitionDigest,
        sim_window_capture_digest: DefinitionDigest,
        target: Option<WindowMetricTarget>,
    ) -> Self {
        Self {
            format: 1,
            projected_room,
            atlas_capture_digest,
            sim_window_capture_digest,
            target,
        }
    }
    pub const fn projected_room(&self) -> ObjId {
        self.projected_room
    }
    pub const fn atlas_capture_digest(&self) -> DefinitionDigest {
        self.atlas_capture_digest
    }
    pub const fn sim_window_capture_digest(&self) -> DefinitionDigest {
        self.sim_window_capture_digest
    }
    pub const fn target(&self) -> Option<WindowMetricTarget> {
        self.target
    }
    pub(super) fn validate(
        &self,
        actor: ObjId,
        admitted_room: ObjId,
    ) -> Result<(), ParticipantError> {
        if self.format != 1
            || actor == self.projected_room
            || (self.target.is_none() && self.projected_room != admitted_room)
            || [self.atlas_capture_digest, self.sim_window_capture_digest]
                .iter()
                .any(|digest| digest.as_bytes() == &[0; 32])
        {
            return Err(refused());
        }
        Ok(())
    }
}

pub struct WindowLocomotionIntent {
    pub(super) base: LocomotionIntent,
    seed: Box<super::super::LocomotionAdmission>,
}
impl WindowLocomotionIntent {
    pub(super) fn new(
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        admission: &super::super::LocomotionAdmission,
    ) -> Result<Self, ParticipantError> {
        exact_window_action(action)?;
        Ok(Self {
            base: LocomotionIntent::new_bound(world, invocation, action, commit, admission)?,
            seed: Box::new(admission.preview_seed()),
        })
    }
    pub fn actor(&self) -> ObjId {
        self.base.actor()
    }
    pub const fn source(&self) -> Option<ObjId> {
        self.base.source()
    }
    pub const fn exit(&self) -> Option<ObjId> {
        self.base.exit()
    }
    pub const fn destination(&self) -> ObjId {
        self.base.destination()
    }
    pub const fn prepared_at(&self) -> &ClockStamp {
        self.base.prepared_at()
    }
    pub const fn at_utc(&self) -> i64 {
        self.base.at_utc()
    }
    pub const fn commit(&self) -> CommitToken {
        self.base.commit()
    }
    pub const fn action(&self) -> &DefinitionRef {
        self.base.action()
    }
    /// Only the exact real owner context can use this Core-minted read seed.
    pub fn preview_projection(
        &self,
        ctx: &OwnerPrepareContext<'_>,
        dependencies: &BTreeSet<ObjId>,
        projected_room: ObjId,
    ) -> Result<LocomotionPreview, ParticipantError> {
        if !self.base.matches_context(ctx)
            || ctx.world().world_ic_now() != *self.prepared_at()
            || ctx.world().now() != self.at_utc()
        {
            return Err(refused());
        }
        self.seed
            .preview_projection(ctx.world(), dependencies, projected_room)
    }
}

pub struct WindowLocomotionDraft {
    pub(super) base: LocomotionDraft,
    pub(super) projection: WindowProjection,
}
impl WindowLocomotionDraft {
    pub fn new(
        ctx: &OwnerPrepareContext<'_>,
        intent: &WindowLocomotionIntent,
        plan_digest: DefinitionDigest,
        capture_digest: DefinitionDigest,
        dependencies: BTreeSet<ObjId>,
        projection: WindowProjection,
    ) -> Result<Self, ParticipantError> {
        projection.validate(intent.actor(), intent.destination())?;
        Ok(Self {
            base: LocomotionDraft::new(
                ctx,
                &intent.base,
                plan_digest,
                capture_digest,
                dependencies,
            )?,
            projection,
        })
    }
}
