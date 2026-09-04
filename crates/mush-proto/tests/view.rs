use mush_proto::{
    MAX_VIEW_DATA_BYTES, MAX_VIEW_WIRE_INTEGER, ViewCursor, ViewEnvelope, ViewEnvelopeError,
    ViewFrameDisposition,
};
use serde_json::json;

fn cursor(generation: u64, sequence: u64) -> ViewCursor {
    ViewCursor {
        view: "legal.actions".into(),
        schema_version: 1,
        generation,
        sequence,
    }
}

#[test]
fn keyframes_replace_but_never_roll_back_one_generation() {
    let initial = ViewEnvelope::keyframe("legal.actions", 1, 7, 3, json!([])).unwrap();
    assert_eq!(
        initial.disposition("legal.actions", 1, None),
        ViewFrameDisposition::Replace
    );
    assert_eq!(initial.cursor(), cursor(7, 3));
    assert_eq!(
        initial.disposition("legal.actions", 1, Some(&cursor(7, 3))),
        ViewFrameDisposition::IgnoreStale
    );
    assert_eq!(
        initial.disposition("legal.actions", 1, Some(&cursor(7, 4))),
        ViewFrameDisposition::IgnoreStale
    );

    let restarted = ViewEnvelope::keyframe("legal.actions", 1, 8, 0, json!([])).unwrap();
    assert_eq!(
        restarted.disposition("legal.actions", 1, Some(&cursor(7, 99))),
        ViewFrameDisposition::Replace
    );
}

#[test]
fn deltas_require_the_exact_generation_and_base_cursor() {
    let delta = ViewEnvelope::delta("legal.actions", 1, 7, 4, 3, json!({"remove": 0})).unwrap();
    assert_eq!(
        delta.disposition("legal.actions", 1, Some(&cursor(7, 3))),
        ViewFrameDisposition::ApplyDelta
    );
    assert_eq!(
        delta.disposition("legal.actions", 1, None),
        ViewFrameDisposition::ResyncRequired
    );
    assert_eq!(
        delta.disposition("legal.actions", 1, Some(&cursor(7, 2))),
        ViewFrameDisposition::ResyncRequired,
        "a dropped predecessor must force a keyframe"
    );
    assert_eq!(
        delta.disposition("legal.actions", 1, Some(&cursor(8, 3))),
        ViewFrameDisposition::ResyncRequired,
        "projection-generation mismatches must force a keyframe"
    );
    assert_eq!(
        delta.disposition("legal.actions", 2, Some(&cursor(7, 3))),
        ViewFrameDisposition::ResyncRequired,
        "unknown schemas must force a keyframe"
    );
    assert_eq!(
        delta.disposition("legal.actions", 1, Some(&cursor(7, 4))),
        ViewFrameDisposition::IgnoreStale,
        "an already-applied delta must not roll state back"
    );
}

#[test]
fn construction_and_deserialization_enforce_bounds_and_sequence_shape() {
    assert!(matches!(
        ViewEnvelope::keyframe("Bad View", 1, 1, 0, json!({})),
        Err(ViewEnvelopeError::InvalidViewByte { .. })
    ));
    assert!(matches!(
        ViewEnvelope::delta("legal.actions", 1, 1, 4, 4, json!({})),
        Err(ViewEnvelopeError::InvalidDeltaSequence {
            base: 4,
            sequence: 4
        })
    ));
    assert!(matches!(
        ViewEnvelope::keyframe("legal.actions", 1, MAX_VIEW_WIRE_INTEGER + 1, 0, json!({}),),
        Err(ViewEnvelopeError::IntegerTooLarge {
            field: "generation",
            ..
        })
    ));
    assert!(matches!(
        ViewEnvelope::keyframe(
            "legal.actions",
            1,
            1,
            0,
            json!("x".repeat(MAX_VIEW_DATA_BYTES + 1)),
        ),
        Err(ViewEnvelopeError::DataTooLarge { .. })
    ));

    let malformed = r#"{
        "view":"legal.actions",
        "schema_version":1,
        "generation":1,
        "sequence":2,
        "base_sequence":2,
        "data":[]
    }"#;
    assert!(serde_json::from_str::<ViewEnvelope>(malformed).is_err());
}
