use super::*;
use mush_core::{ModuleBootData, mechanics::DefinitionDigest};
use mush_roll::{Envelope, RevisionKey, commerce::RevisionIdentity};
use serde_json::json;

struct Sources(String);
impl SourceAdapter for Sources {
    type Token = ();
    fn prepare_source(&self, _: &str, _: &str) -> Result<mush_roll::PreparedSource<()>, RollError> {
        Err(RollError::Missing)
    }
    fn read_source(&self, digest: DefinitionDigest) -> Result<&str, RollError> {
        if mush_roll::source_digest(&self.0) == digest {
            Ok(&self.0)
        } else {
            Err(RollError::Missing)
        }
    }
}

#[test]
fn new_draft_execution_and_trace_refuse_marked_revision_but_frozen_clone_is_unchanged()
-> Result<(), Box<dyn std::error::Error>> {
    let sources = Sources("fn think(view, card) {}".into());
    let actor = mush_core::GOD;
    let envelope = Envelope {
        schema: json!({"sections":[]}),
        presets: json!({}),
        profiles: json!({"Default":{}}),
        rules: json!({}),
        declared_class: "I".into(),
        mind_abi: crate::minds::MIND_ABI.into(),
    };
    let reference = mush_roll::RevisionRef {
        key: RevisionKey {
            character: actor,
            mind: 1,
            number: 1,
        },
        digest: envelope.digest(&sources.0)?,
    };
    let original: Revision = serde_json::from_value(
        json!({"format":1,"reference":reference,"source":mush_roll::source_digest(&sources.0),"envelope":envelope,"notes":""}),
    )?;
    let frozen = SavedMind::from_revision(&sources, actor, "owned", &original)?;
    let before = frozen.clone();
    let parent_ref = mush_roll::RevisionRef {
        key: RevisionKey {
            character: ObjId(99),
            mind: 1,
            number: 1,
        },
        digest: reference.digest,
    };
    let mut parent_value = serde_json::to_value(&original)?;
    parent_value
        .as_object_mut()
        .ok_or("object")?
        .insert("reference".into(), json!(parent_ref));
    let parent: Revision = serde_json::from_value(parent_value)?;
    let mut marked_value = serde_json::to_value(&original)?;
    let object = marked_value.as_object_mut().ok_or("object")?;
    object.insert("format".into(), json!(2));
    object.insert("provenance".into(),json!({"parent":{"filed":"1","revision":RevisionIdentity::capture(&parent)?},"charter":"90","line":"1"}));
    let marked: Revision = serde_json::from_value(marked_value.clone())?;
    marked.verify_source(&sources)?;
    check_eq!(marked.reference(), original.reference());
    check!(matches!(
        SavedMind::from_revision(&sources, actor, "owned", &marked),
        Err(RollError::Missing)
    ));
    check_eq!(frozen, before);
    let original_store = RollStore::load(&ModuleBootData {
        kv: vec![(
            format!("revision:{}:1:1", actor.0).into(),
            serde_json::to_value(original)?,
        )],
        ..Default::default()
    })?;
    check!(current_draft_source_access(
        Some(&original_store),
        &frozen.origin
    ));
    let marked_store = RollStore::load(&ModuleBootData {
        kv: vec![
            (format!("revision:{}:1:1", actor.0).into(), marked_value),
            ("revision:99:1:1".into(), serde_json::to_value(parent)?),
        ],
        ..Default::default()
    })?;
    check_eq!(
        marked_store.revision(actor, reference.key)?.reference(),
        reference
    );
    check!(!current_draft_source_access(
        Some(&marked_store),
        &frozen.origin
    ));
    check!(!current_draft_source_access(None, &frozen.origin));
    check!(current_draft_source_access(
        None,
        &MindOrigin::Doctrine { revision: 1 }
    ));
    Ok(())
}
