#![cfg(test)]

//! Real registered Core coordinator evidence. Value codecs are deliberately
//! inert; this proves capacity accounting, not physical/E4 business authority.
use super::*;

fn completed(reverse: bool) -> TestResult<(Fixture, Value)> {
    let (mut f, raw) = accepted_profile(reverse, true)?;
    let (reserved, writes) = super::reserved(&f, &raw)?;
    let p = prepare(&mut f, 1, writes, 0, token(2), true, ObjId(7), None)?;
    install(&mut f, p)?;
    let (next, writes) = terminal(&f, &reserved, true, token(3))?;
    let p = prepare(&mut f, 3, writes, 0, token(3), true, ObjId(7), None)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    Ok((f, next))
}
fn deliver(f: &Fixture, raw: &Value, op: CommitToken) -> TestResult<Vec<Mutation>> {
    let r = Row::parse(raw.clone(), 0)?;
    let output = current(f, &key(OUTPUT, "0".into())?)?;
    let action = f
        .defs
        .delivery
        .clone()
        .ok_or("actual delivery definition")?;
    let core = json!({"format":1,"operation":op,"action":action,"output":output,
        "at_utc":-9,"delivered_digest":dg(70)});
    let digest = DefinitionDigest::for_canonical_value("industry.core_delivery.receipt.v1", &core)?;
    let industry = json!({"format":1,"sequence":1,"order":0,"accepted":r.accepted,
        "completion":token(3),"operation":op,"action":action,"object":200,
        "reservation":token(2),"at_utc":-9,"core_receipt":digest});
    Ok(vec![
        put(order_key(0)?, raw.clone()),
        put(key(delivery::CORE, "0".into())?, core),
        put(
            key(delivery::INDUSTRY, "0000000000000000".into())?,
            industry,
        ),
    ])
}
fn remove_family(disk: &mut Disk, family: &str) -> TestResult {
    let k = disk
        .iter()
        .find(|(_, v)| v.pointer("/family/id") == Some(&json!(family)))
        .map(|(k, _)| k.clone())
        .ok_or("exact family row")?;
    disk.remove(&k);
    Ok(())
}
#[test]
fn actual_paid_profile_retains_same_admission_until_both_delivery_receipts_install() -> TestResult {
    for reverse in [false, true] {
        let (mut f, raw) = accepted_profile(reverse, true)?;
        let before = metadata(&f).pop().ok_or("funding metadata")?;
        assert_eq!(before["targets"].as_array().ok_or("targets")?.len(), 12);
        let (reserved, writes) = super::reserved(&f, &raw)?;
        let p = prepare(&mut f, 1, writes, 0, token(2), true, ObjId(7), None)?;
        install(&mut f, p)?;
        let (next, writes) = terminal(&f, &reserved, true, token(3))?;
        let p = prepare(&mut f, 3, writes, 0, token(3), true, ObjId(7), None)?;
        install(&mut f, p)?;
        let pending = metadata(&f).pop().ok_or("retained delivery metadata")?;
        assert_eq!(pending["targets"].as_array().ok_or("targets")?.len(), 3);
        for name in [
            "format",
            "funding",
            "effect",
            "sponsor",
            "protocol_fingerprint",
        ] {
            assert_eq!(before.get(name), pending.get(name));
        }
        assert!(
            canonical_json_bytes_with_limit(&pending, 65536)?.len()
                < canonical_json_bytes_with_limit(&before, 65536)?.len()
        );
        let r = Row::parse(next.clone(), 0)?;
        let keys = r.keys()?;
        assert!(
            f.engine
                .action_state()
                .get(
                    &f.handles.lock().unwrap()[SCHEDULE],
                    &keys[SCHEDULE].encoded.to_string()
                )?
                .is_none()
        );
        hydrate(&f, &f.disk)?;
        let before_usage = f.engine.action_state().capacity_usage;
        let writes = deliver(&f, &next, token(4))?;
        let p = prepare(&mut f, 6, writes, 0, token(4), true, ObjId(7), None)?;
        install(&mut f, p)?;
        assert!(metadata(&f).is_empty());
        let after_usage = f.engine.action_state().capacity_usage;
        assert!(after_usage.bytes <= before_usage.bytes);
        assert!(after_usage.records <= before_usage.records);
        // This exact transition remains inside either global limit even when
        // unrelated admitted state has already consumed all remaining room.
        assert!(
            Usage {
                bytes: MAX_ACTION_STATE_BOOT_BYTES - before_usage.bytes + after_usage.bytes,
                records: MAX_ACTION_STATE_BOOT_RECORDS - before_usage.records + after_usage.records
            }
            .check()
            .is_ok()
        );
        hydrate(&f, &f.disk)?;
    }
    Ok(())
}
#[test]
fn delivery_promise_cancel_drains_before_or_after_reserve_under_changed_recipe() -> TestResult {
    for reserve in [false, true] {
        let (mut old, mut raw) = accepted_profile(false, true)?;
        if reserve {
            let (next, writes) = super::reserved(&old, &raw)?;
            let p = prepare(&mut old, 1, writes, 0, token(2), true, ObjId(7), None)?;
            install(&mut old, p)?;
            raw = next;
        }
        let mut f = fixture_profile(2, true, true, 0, true)?;
        f.engine.hydrate_action_state(
            old.disk
                .iter()
                .filter(|((_, k), _)| k.starts_with("action-state/"))
                .map(|((n, k), v)| ActionStateBootRecord::new(n.clone(), k.clone(), v.clone())),
        )?;
        f.disk = old.disk;
        let (_, writes) = terminal(&f, &raw, false, token(4))?;
        let p = prepare(&mut f, 4, writes, 0, token(4), true, ObjId(7), None)?;
        install(&mut f, p)?;
        assert!(metadata(&f).is_empty());
        assert!(
            !f.disk
                .values()
                .any(|v| v.pointer("/family/id") == Some(&json!(delivery::CORE)))
        );
        hydrate(&f, &f.disk)?;
    }
    Ok(())
}
#[test]
fn completed_delivery_drains_after_recipe_change_and_requires_actual_requester() -> TestResult {
    let (old, raw) = completed(false)?;
    let mut f = fixture_profile(2, true, false, 0, true)?;
    f.engine.hydrate_action_state(
        old.disk
            .iter()
            .filter(|((_, k), _)| k.starts_with("action-state/"))
            .map(|((n, k), v)| ActionStateBootRecord::new(n.clone(), k.clone(), v.clone())),
    )?;
    f.disk = old.disk;
    let writes = deliver(&f, &raw, token(4))?;
    assert!(prepare(&mut f, 6, writes, 0, token(4), true, ObjId(8), None).is_err());
    let writes = deliver(&f, &raw, token(4))?;
    let p = prepare(&mut f, 6, writes, 0, token(4), true, ObjId(7), None)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    Ok(())
}
#[test]
fn delivery_requires_both_exact_rows_marker_and_current_action_commit() -> TestResult {
    for mode in 0..7 {
        let (mut f, raw) = completed(false)?;
        let mut writes = deliver(&f, &raw, token(4))?;
        let mut index = 6;
        let mut commit = token(4);
        let mut marker = true;
        match mode {
            0 => {
                writes.pop();
            }
            1 => {
                writes.remove(0);
            }
            2 => index = 5,
            3 => commit = token(6),
            4 => marker = false,
            5 => {
                writes
                    .get_mut(2)
                    .ok_or("industry row")?
                    .value
                    .as_mut()
                    .ok_or("value")?["completion"] = json!(token(88));
            }
            _ => {
                writes
                    .get_mut(0)
                    .ok_or("order")?
                    .value
                    .as_mut()
                    .ok_or("value")?["sequence"] = json!(2);
            }
        }
        let before = f.disk.clone();
        assert!(
            prepare(&mut f, index, writes, 0, commit, marker, ObjId(7), None).is_err(),
            "closed branch {mode}"
        );
        assert!(f.disk == before, "refusal changed acknowledged fixture");
        hydrate(&f, &f.disk)?;
    }
    Ok(())
}
#[test]
fn cold_missing_obligation_or_partial_and_repaired_crossed_receipts_refuse() -> TestResult {
    let (mut f, raw) = completed(false)?;
    let mut missing = f.disk.clone();
    missing.retain(|(_, k), _| !k.starts_with(PREFIX));
    assert!(hydrate(&f, &missing).is_err());
    let legacy = fixture(1, true, false)?;
    assert!(hydrate(&legacy, &f.disk).is_err());
    let writes = deliver(&f, &raw, token(4))?;
    let p = prepare(&mut f, 6, writes, 0, token(4), true, ObjId(7), None)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    for family in [delivery::CORE, delivery::INDUSTRY] {
        let mut missing = f.disk.clone();
        remove_family(&mut missing, family)?;
        assert!(hydrate(&f, &missing).is_err());
    }
    let mut missing = f.disk.clone();
    remove_family(&mut missing, delivery::CORE)?;
    remove_family(&mut missing, delivery::INDUSTRY)?;
    assert!(hydrate(&f, &missing).is_err());
    let mut crossed = f.disk.clone();
    let core = crossed
        .values_mut()
        .find(|v| v.pointer("/family/id") == Some(&json!(delivery::CORE)))
        .ok_or("core row")?;
    core["value"]["output"]["object"] = json!(201);
    let digest =
        DefinitionDigest::for_canonical_value("industry.core_delivery.receipt.v1", &core["value"])?;
    let industry = crossed
        .values_mut()
        .find(|v| v.pointer("/family/id") == Some(&json!(delivery::INDUSTRY)))
        .ok_or("Industry row")?;
    industry["value"]["object"] = json!(201);
    industry["value"]["core_receipt"] = json!(digest);
    assert!(hydrate(&f, &crossed).is_err());
    Ok(())
}
#[test]
fn v2_fixed_future_targets_are_prepaid_and_old_profile_wire_is_unchanged() -> TestResult {
    let (f, raw) = accepted_profile(false, true)?;
    let r = Row::parse(raw, 0)?;
    let all = targets(&r)?;
    assert_eq!(all.len(), 12);
    assert_eq!(
        all.iter().map(|t| t.ceiling).sum::<usize>(),
        354304 + 16384 + 4096
    );
    assert_eq!(all.iter().filter(|t| t.mode == SlotMode::Create).count(), 2);
    let view = View {
        registry: &f.engine.action_state().registry,
        records: &f.engine.action_state().records,
        mutations: None,
    };
    require_future_room(&view, &r)?;
    let old = defs(1)?;
    let protocol = Protocol::new(
        &owner_id("industry")?,
        old.participants.clone(),
        old.actions.clone(),
    )?;
    let wire = serde_json::to_value(protocol)?;
    assert_eq!(
        wire,
        json!({"participants":old.participants,"actions":old.actions})
    );
    assert!(delivery::promised(&json!({"delivery":null})).is_err());
    assert!(delivery::promised(&json!({"delivery":"unknown"})).is_err());
    let (legacy, raw) = accepted(false)?;
    assert_eq!(targets(&Row::parse(raw, 0)?)?.len(), 10);
    hydrate(&legacy, &legacy.disk)?;
    Ok(())
}
