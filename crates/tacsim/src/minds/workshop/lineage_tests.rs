//! Structural marked-record fixtures only: no parent grant or Open fork issuer.
use super::*;
use mush_core::{ModuleBootData, mechanics::DefinitionDigest};
use mush_roll::{Mind, Revision, RevisionKey, RevisionRef, commerce::RevisionIdentity};
use serde_json::json;
use std::collections::BTreeMap;

type TestResult = Result<(), Box<dyn std::error::Error>>;

pub(super) fn marked_store(mind: &Mind) -> Result<RollStore, Box<dyn std::error::Error>> {
    let actor = mush_core::GOD;
    let parent_actor = ObjId(999);
    let parent_ref = RevisionRef {
        key: RevisionKey {
            character: parent_actor,
            mind: 1,
            number: 1,
        },
        digest: mind.draft().envelope.digest(&mind.draft().source)?,
    };
    let parent: Revision = serde_json::from_value(json!({
        "format":1,"reference":parent_ref,"source":mush_roll::source_digest(&mind.draft().source),
        "envelope":mind.draft().envelope,"notes":""
    }))?;
    let mut row = serde_json::to_value(mind)?;
    let object = row.as_object_mut().ok_or("mind object")?;
    object.insert("format".into(), json!(2));
    object.insert(
        "provenance".into(),
        json!({
            "parent":{"filed":"1","revision":RevisionIdentity::capture(&parent)?},
            "charter":"90","line":"1"
        }),
    );
    let mut kv: BTreeMap<smol_str::SmolStr, Value> = BTreeMap::new();
    let names = BTreeMap::from([(mind.name().to_owned(), mind.id())]);
    kv.insert(
        format!("library:{}", actor.0).into(),
        json!({
            "format":1,"character":actor,"next_mind":mind.id()+1,
            "names":names,"selection":null
        }),
    );
    kv.insert(format!("mind:{}:{}", actor.0, mind.id()).into(), row);
    kv.insert("revision:999:1:1".into(), serde_json::to_value(parent)?);
    let store = RollStore::load(&ModuleBootData {
        kv: kv.into_iter().collect(),
        ..Default::default()
    })?;
    // The fixture preserves the exact original CAS/content, and must remain a
    // readable owner record; adapter refusal must be provenance, not corruption.
    check_eq!(store.mind(actor, mind.name())?.base(), mind.base());
    check_eq!(store.mind(actor, mind.name())?.draft(), mind.draft());
    Ok(store)
}

pub(super) struct Sources;
impl SourceAdapter for Sources {
    type Token = ();
    fn prepare_source(&self, _: &str, _: &str) -> Result<mush_roll::PreparedSource<()>, RollError> {
        Err(RollError::Missing)
    }
    fn read_source(&self, _: DefinitionDigest) -> Result<&str, RollError> {
        Err(RollError::Missing)
    }
}

#[test]
fn marked_source_and_edits_refuse_without_rewriting_the_owned_record() -> TestResult {
    let actor = mush_core::GOD;
    let mut original = RollStore::default();
    let prepared = create(&original, &Sources, actor, "mine", None)?;
    original.publish_with(prepared, |_| {})?;
    let marked = marked_store(original.mind(actor, "mine")?)?;
    let mind = marked.mind(actor, "mine")?;
    let before = serde_json::to_vec(mind)?;
    check_eq!(mind.require_original_access(), Err(RollError::Missing));
    check_eq!(
        owned_copy(&marked, &Sources, actor, &MindRef::parse("mine")?),
        Err(RollError::Missing)
    );
    check!(matches!(
        create(&marked, &Sources, actor, "fork", Some("mine")),
        Err(RollError::Missing)
    ));
    check!(matches!(
        edit_text(
            &marked,
            actor,
            "mine",
            mind.base(),
            TextEdit::ReplaceAll("new".into())
        ),
        Err(RollError::Missing)
    ));
    check_eq!(serde_json::to_vec(marked.mind(actor, "mine")?)?, before);
    Ok(())
}
