//! Current output fences over real loaded owner records. The marked rows are
//! structural fixtures, not an Open grant or a live parent-rights capability.
use super::*;
use crate::arena::profile_seal::MindOrigin;
use mush_core::{GOD, ModuleBootData};
use mush_roll::{Revision, RevisionRef, RollStore, commerce::RevisionIdentity};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn picture(origin: Option<MindOrigin>) -> Picture {
    Picture {
        access_origin: origin,
        unit: ObjId(44),
        event: 7,
        at_ms: 1000,
        complete: false,
        state_hash: "opaque".into(),
        view: json!({"status":"public"}),
        card: Some(json!({"private":"card"})),
        memory: Some(json!({"private":"lineage-memory"})),
        traces: Vec::new(),
        diff: None,
        private_run: true,
        timeline: super::super::super::branch::Timeline::Reference,
        possessed: false,
        refusals: Vec::new(),
    }
}

fn mark(
    documents: &[(smol_str::SmolStr, Value)],
    reference: RevisionRef,
) -> Result<RollStore, Box<dyn std::error::Error>> {
    let mut documents = documents.to_vec();
    let key = format!(
        "revision:{}:{}:{}",
        reference.key.character.0, reference.key.mind, reference.key.number
    );
    let (_, value) = documents
        .iter()
        .find(|(k, _)| k.as_str() == key)
        .ok_or("revision")?;
    let mut parent = value.clone();
    let parent_ref = RevisionRef {
        key: mush_roll::RevisionKey {
            character: ObjId(999),
            mind: 1,
            number: 1,
        },
        digest: reference.digest,
    };
    parent
        .as_object_mut()
        .ok_or("parent object")?
        .insert("reference".into(), json!(parent_ref));
    let parent_revision: Revision = serde_json::from_value(parent.clone())?;
    let provenance = json!({
        "parent":{"filed":"1","revision":RevisionIdentity::capture(&parent_revision)?},
        "charter":"90","line":"1"
    });
    let mind_key = format!("mind:{}:{}", reference.key.character.0, reference.key.mind);
    for (stored, value) in &mut documents {
        if stored.as_str() == key || stored.as_str() == mind_key {
            let object = value.as_object_mut().ok_or("owner object")?;
            object.insert("format".into(), json!(2));
            object.insert("provenance".into(), provenance.clone());
        }
    }
    documents.push(("revision:999:1:1".into(), parent));
    let owner = RollStore::load(&ModuleBootData {
        kv: documents,
        ..Default::default()
    })?;
    check_eq!(owner.revision(GOD, reference.key)?.reference(), reference);
    check_eq!(owner.mind(GOD, "retained")?.latest(), Some(reference.key));
    Ok(owner)
}

#[test]
fn cached_picture_and_pending_candidate_recheck_exact_current_owner_before_output() -> TestResult {
    let (mut engine, service, _, documents) = cleanup_rig(false, false);
    let owner = engine.world.module_state::<RollStore>().ok_or("roll")?;
    let reference = owner
        .revision(GOD, owner.mind(GOD, "retained")?.latest().ok_or("head")?)?
        .reference();
    let origin = MindOrigin::Draft {
        character: GOD,
        mind: reference.key.mind,
        revision: reference.key.number,
        digest: reference.digest,
    };
    let mut cached = picture(Some(origin.clone()));
    check!(
        serde_json::to_value(&cached)?
            .get("access_origin")
            .is_none()
    );
    {
        let mut runtime = service.0.lock().map_err(|_| "runtime")?;
        let row = runtime.entries.values_mut().next().ok_or("entry")?;
        row.picture = Some(Arc::new(cached.clone()));
        check!(authorized(&engine.world, row));
    }
    let visible = cleanup_command(&mut engine, "BENCH PIC");
    check!(format!("{:?}", visible.events()).contains("lineage-memory"));
    engine
        .world
        .module_state_insert::<RollStore>(mark(&documents, reference)?);
    let refused = cleanup_command(&mut engine, "BENCH PIC");
    check!(!format!("{:?}", refused.events()).contains("lineage-memory"));
    check!(
        !refused
            .events()
            .iter()
            .any(|(_, event)| matches!(event, OutputEvent::Panel { .. }))
    );

    let mut runtime = service.0.lock().map_err(|_| "runtime")?;
    let row = runtime
        .entries
        .values_mut()
        .next()
        .ok_or("retained entry")?;
    // A new selected picture must bring its own exact origin, and missing
    // metadata cannot silently inherit the previous unit's authorization.
    cached.unit = ObjId(45);
    cached.access_origin = Some(MindOrigin::Doctrine { revision: 1 });
    row.picture = Some(Arc::new(cached.clone()));
    check!(source_access(&engine.world, row));
    row.pending_revision = Some(reference);
    check!(
        !source_access(&engine.world, row),
        "delayed candidate result"
    );
    row.source_revision = row.pending_revision.take();
    check!(
        !source_access(&engine.world, row),
        "retained fork/diff result"
    );
    row.source_revision = None;
    cached.access_origin = None;
    row.picture = Some(Arc::new(cached));
    check!(!source_access(&engine.world, row));
    check!(engine.take_module_ops().is_empty());
    Ok(())
}
