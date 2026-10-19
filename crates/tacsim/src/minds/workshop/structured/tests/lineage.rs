use super::super::super::lineage_tests::marked_store;
use super::*;
use mush_roll::RollStore;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn mark(engine: &mut Engine) -> TestResult {
    let marked = marked_store(
        engine
            .world
            .module_state::<RollStore>()
            .ok_or("roll")?
            .mind(GOD, "mine")?,
    )?;
    engine.world.module_state_insert::<RollStore>(marked);
    Ok(())
}

#[test]
fn invoke_cannot_read_replace_copy_check_or_save_a_marked_owned_draft() -> TestResult {
    let (mut engine, _, _) = rig();
    let base = create(&mut engine);
    let MindResponse::Read { draft, .. } = invoke(
        &mut engine,
        7,
        GOD,
        80,
        MindRequest::Read {
            name: "mine".into(),
            revision: None,
        },
    ) else {
        return Err("read original".into());
    };
    mark(&mut engine)?;
    let before = serde_json::to_value(
        engine
            .world
            .module_state::<RollStore>()
            .ok_or("roll")?
            .mind(GOD, "mine")?,
    )?;
    for (index, request) in [
        MindRequest::Read {
            name: "mine".into(),
            revision: None,
        },
        MindRequest::Create {
            name: "copy".into(),
            from: Some("mine".into()),
        },
        MindRequest::Replace {
            name: "mine".into(),
            base,
            draft: draft.clone(),
        },
        MindRequest::Check {
            name: "mine".into(),
            base,
        },
        MindRequest::Save {
            name: "mine".into(),
            base,
            notes: String::new(),
            version: None,
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            matches!(invoke(&mut engine,7,GOD,81+u64::try_from(index)?,request),MindResponse::Error{code,..} if code=="missing")
        );
    }
    assert_eq!(
        serde_json::to_value(
            engine
                .world
                .module_state::<RollStore>()
                .ok_or("roll")?
                .mind(GOD, "mine")?
        )?,
        before
    );
    let response = invoke(
        &mut engine,
        7,
        GOD,
        90,
        MindRequest::Lifecycle {
            name: "mine".into(),
            base,
            edit: mush_proto::MindLifecycle::Rename {
                new_name: "renamed".into(),
            },
        },
    );
    assert!(matches!(response, MindResponse::Edited { .. }));
    let renamed = engine
        .world
        .module_state::<RollStore>()
        .ok_or("roll")?
        .mind(GOD, "renamed")?;
    assert_eq!(
        serde_json::to_value(renamed)?.get("provenance"),
        before.get("provenance")
    );
    assert_eq!(renamed.require_original_access(), Err(RollError::Missing));
    Ok(())
}

#[test]
fn queued_check_and_save_recheck_lineage_before_any_remote_delivery() -> TestResult {
    for save in [false, true] {
        let (mut engine, service, _) = rig();
        let base = create(&mut engine);
        let request = if save {
            MindRequest::Save {
                name: "mine".into(),
                base,
                notes: String::new(),
                version: None,
            }
        } else {
            MindRequest::Check {
                name: "mine".into(),
                base,
            }
        };
        assert!(matches!(
            invoke(&mut engine, 7, GOD, 95, request),
            MindResponse::Queued { .. }
        ));
        await_ready(&service);
        // Same current character, connection, CAS and all executable bytes.
        // Only retained owner provenance changes, so base equality is not ACL.
        mark(&mut engine)?;
        let out = poll(&mut engine, &service);
        assert!(
            matches!(out.events(),[(Audience::Conn(7),OutputEvent::InvokeResult{id:95,result:InvokeResult::Mind(MindResponse::Error{code,..})})] if code=="missing")
        );
        assert!(
            engine
                .world
                .module_state::<RollStore>()
                .ok_or("roll")?
                .mind(GOD, "mine")?
                .latest()
                .is_none()
        );
        assert!(service.executor.ready().is_empty());
        assert!(
            poll(&mut engine, &service).events().is_empty(),
            "one-shot delivery"
        );
    }
    Ok(())
}
