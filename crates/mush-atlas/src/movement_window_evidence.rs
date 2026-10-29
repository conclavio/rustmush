//! Historical data validation only. No wire can manufacture an executable
//! WindowArrivalPlan, recheck seed, owner preparation or placement capability.

use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    capture: Value,
    #[serde(deserialize_with = "required_option")]
    placement: Option<Placement>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    domain: u64,
    portal: Portal,
    geometry: GeometryObservation,
    #[serde(deserialize_with = "required_option")]
    occupancy: Option<OccupancyObservation>,
    outcome: EntryOutcome,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Portal {
    probes: usize,
    digest: DefinitionDigest,
    #[serde(deserialize_with = "required_option")]
    selected: Option<RecordKey>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}

pub(crate) fn validate_evidence(
    value: &Value,
    projection: &WindowProjection,
) -> Result<MovementCapture, CaptureError> {
    preflight_evidence(value)?;
    canonical_json_bytes_with_limit(value, MAX_WINDOW_PLAN_BYTES)
        .map_err(|_| CaptureError::Limit)?;
    let wire: Wire = serde_json::from_value(value.clone()).map_err(|_| CaptureError::Invalid)?;
    if wire.format != 1
        || serde_json::to_value(&wire).map_err(|_| CaptureError::Encoding)? != *value
    {
        return Err(CaptureError::Invalid);
    }
    let bytes =
        canonical_json_bytes_with_limit(&wire.capture, crate::movement::MAX_MOVEMENT_CAPTURE_BYTES)
            .map_err(|_| CaptureError::Limit)?;
    let capture = MovementCapture::decode(&bytes)?;
    if let Some(placement) = &wire.placement
        && (placement.portal.probes > MAX_WINDOW_PLAN_PORTAL_PROBES
            || (placement.portal.probes == 0 && placement.portal.selected.is_some())
            || !matches!(placement.geometry, GeometryObservation::Drawn { .. })
            || placement
                .occupancy
                .as_ref()
                .is_none_or(|o| o.records > MAX_WINDOW_PLAN_PLACEMENTS)
            || !matches!(placement.outcome, EntryOutcome::Selected { surface, .. } if surface < MAX_WINDOW_PLAN_SURFACES))
    {
        return Err(CaptureError::Invalid);
    }
    let (room, target) = proposal_target(
        &capture,
        wire.placement.as_ref().map(|p| (p.domain, &p.outcome)),
    )?;
    if WindowProjection::new(
        room,
        digest(
            "atlas.window_arrival.capture.v1",
            value,
            MAX_WINDOW_PLAN_BYTES,
        )?,
        projection.sim_window_capture_digest(),
        target,
    ) != *projection
    {
        return Err(CaptureError::Invalid);
    }
    Ok(capture)
}

/// A byte cap alone cannot bound recursion before a clone or serde call.
pub(crate) fn preflight_evidence(value: &Value) -> Result<(), CaptureError> {
    let mut pending = vec![(value, 0_usize)];
    let mut nodes = 1_usize;
    let mut text = MAX_WINDOW_PLAN_BYTES;
    while let Some((value, depth)) = pending.pop() {
        if depth > 32 {
            return Err(CaptureError::Limit);
        }
        let children = match value {
            Value::Array(values) => values.len(),
            Value::Object(values) => values.len(),
            _ => 0,
        };
        nodes = nodes
            .checked_add(children)
            .filter(|nodes| *nodes <= 65_536)
            .ok_or(CaptureError::Limit)?;
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|v| (v, depth + 1))),
            Value::Object(values) => {
                for (key, value) in values {
                    text = text.checked_sub(key.len()).ok_or(CaptureError::Limit)?;
                    pending.push((value, depth + 1));
                }
            }
            Value::String(value) => {
                text = text.checked_sub(value.len()).ok_or(CaptureError::Limit)?;
            }
            _ => {}
        }
    }
    Ok(())
}
