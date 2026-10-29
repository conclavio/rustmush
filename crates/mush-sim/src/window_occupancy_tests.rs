#![cfg(test)]

use super::*;
use crate::{PlacementRecord, Pose};
use serde_json::json;

fn placed(object: u32, domain: u64, x_cm: i64, y_cm: i64) -> PlacementRecord {
    PlacementRecord {
        object: ObjId(object),
        parent: SpatialParent::Direct {
            domain: DomainId(domain),
            pose: Pose::at(
                x_cm,
                y_cm,
                0,
                crate::passive_rules_value(json!({})).expect("pose"),
            ),
        },
        revision: 1,
        unplaced: None,
    }
}

#[test]
fn rows_keep_map_order_and_both_identities_without_reading_opaque_postures() {
    let mut state = SpatialState::default();
    state.placements.insert(ObjId(9), placed(10, 7, -4, 5));
    state.placements.insert(ObjId(3), placed(2, 7, 6, -7));
    state.placements.insert(ObjId(8), placed(8, 8, i64::MIN, 0));
    let selected = rows(&state, DomainId(7)).expect("bounded rows");
    assert_eq!(
        selected
            .iter()
            .map(|r| (r.key, r.object))
            .collect::<Vec<_>>(),
        vec![(ObjId(3), ObjId(2)), (ObjId(9), ObjId(10))]
    );
    let fence = OccupancyFence::capture(&state, DomainId(7)).expect("capture");
    if let SpatialParent::Direct { pose, .. } =
        &mut state.placements.get_mut(&ObjId(3)).expect("row").parent
    {
        pose.z_cm = i32::MAX;
        pose.heading_mdeg = 90_000;
        pose.posture = crate::passive_rules_value(json!({"unconsulted":"x".repeat(512 * 1024)}))
            .expect("posture");
    }
    assert!(fence.recheck(&state));
}

#[test]
fn coordinate_and_global_scan_limits_are_checked_before_collection() {
    let mut state = SpatialState::default();
    state
        .placements
        .insert(ObjId(1), placed(1, 7, -1_000_000_000, 1_000_000_000));
    assert!(rows(&state, DomainId(7)).is_ok());
    for (x, y) in [
        (1_000_000_001, 0),
        (0, -1_000_000_001),
        (i64::MIN, 0),
        (0, i64::MAX),
    ] {
        state.placements.insert(ObjId(1), placed(1, 7, x, y));
        assert_eq!(rows(&state, DomainId(7)), Err(OccupancyError::Coordinate));
    }
    state.placements = (0..4096).map(|n| (ObjId(n), placed(n, 8, 0, 0))).collect();
    assert!(
        rows(&state, DomainId(7))
            .expect("4096 rows scanned")
            .is_empty()
    );
    state.placements.insert(ObjId(4096), placed(4096, 8, 0, 0));
    assert_eq!(rows(&state, DomainId(7)), Err(OccupancyError::Limit));
}
