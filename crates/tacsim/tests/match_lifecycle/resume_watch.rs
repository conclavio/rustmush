//! Actual lobby -> native Order -> durable snapshot -> ordinary boot resume.
//! No watch is injected into either registry. Fine clock/recipient assertions are
//! in the private bridge tests; this pins the real boot entry and attribution.
use super::*;
use tacsim::deliberation::{
    CharacterRef,
    order::{Constraints, Objective, Order},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r4_actual_boot_restores_accepted_character_and_persona_attribution() {
    for persona in [false, true] {
        let mut cfg = af3_config();
        cfg.match_.ai_fill = true;
        let mut rig = Rig::boot_on(2, roster_with(cfg.clone()));
        // Enabling fill alone defaults to the larger human side (one here),
        // which creates no machine squad. Request real unclaimed fill pods.
        rig.tick_to(1);
        rig.say(0, "sim team red");
        rig.say(1, "sim team blue");
        assert!(rig.say(0, "sim ai 2").contains("AI fill set to 2 a side."));
        assert!(rig.say(0, "sim start").contains("boarding"));
        let (said, _) = rig.board_writes(0, match_::RED_ROOM_ENTRY);
        assert!(said.contains("climb into"), "actual red BOARD");
        let (said, _) = rig.board_writes(1, match_::BLUE_ROOM_ENTRY);
        assert!(said.contains("climb into"), "actual blue BOARD");
        assert_eq!(rig.state(), MatchState::Running);
        let match_id = rig.centre().match_id.expect("actual running filled match");
        let record = rig.book().get(match_id).expect("running record");
        let squad = record
            .squads
            .iter()
            .find(|s| s.id.team == Team::Red)
            .expect("actual fill squad")
            .id;
        let expected = if persona {
            CharacterRef::Persona { squad }
        } else {
            CharacterRef::Character { id: rig.pilot(0) }
        };
        if persona {
            assert_eq!(
                tacsim::deliberation::leader_of(&record.leaders, squad),
                Some(&expected)
            );
            let reply = tacsim::deliberation::DeliberationReply::Ok {
                match_id: match_id.0,
                order: Order {
                    squad,
                    objective: Objective::Hold,
                    target: None,
                    horizon_ms: 60000,
                    constraints: Constraints::default(),
                    ttl_ms: 60000,
                    issued_ms: 0,
                    issuer: None,
                    by: expected.clone(),
                    spot: None,
                },
                model: "fixture".into(),
                prompt_hash: "accepted-order-recovery".into(),
            };
            let payload = serde_json::to_string(&reply).expect("reply wire");
            let mut out = Outbox::new();
            let engine = rig.tw.engine_mut();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(tacsim::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            tacsim::arena_bridge::on_deliberation_reply(&mut ctx, &payload);
        } else {
            let said = rig.say(0, &format!("ORDER {squad} HOLD"));
            assert!(said.contains("Ordered"), "actual public order accepted");
        }
        // Wait for a real snapshot that includes the actual native acceptance,
        // rather than mistaking the earlier world offer watch for acceptance.
        let key = match_::snapshot_key(match_id).to_string();
        let mut captured = false;
        for _ in 0..1500 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            rig.pump();
            let _ = rig.pump_ops();
            if let Some(document) = rig.image_doc(&key) {
                let state = document
                    .get("state")
                    .and_then(serde_json::Value::as_str)
                    .expect("snapshot state");
                let value: serde_json::Value = serde_json::from_str(state).expect("snapshot tree");
                if value
                    .get("order_watches")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|rows| {
                        rows.iter().any(|row| {
                            row.get(0) == Some(&serde_json::json!(squad))
                                && row.get(1).and_then(|g| g.get("by"))
                                    == Some(&serde_json::json!(expected))
                                && row
                                    .get(1)
                                    .and_then(|g| g.get("members"))
                                    .and_then(serde_json::Value::as_array)
                                    .is_some_and(|members| !members.is_empty())
                        })
                    })
                {
                    captured = true;
                    break;
                }
            }
        }
        assert!(
            captured,
            "actual acceptance must reach acknowledged snapshot"
        );
        let issuer = rig.registry().order_issuer(match_id, squad);
        let before = rig.registry().order_author(match_id, squad);
        assert_eq!(before, Some(expected.clone()));
        reboot_fresh(&mut rig, cfg).await;
        assert!(
            rig.registry().tasks().is_live(match_id),
            "actual resume task"
        );
        assert_eq!(rig.registry().order_author(match_id, squad), Some(expected));
        assert_eq!(rig.registry().order_issuer(match_id, squad), issuer);
        assert_eq!(
            rig.registry().deliberation_counts(match_id),
            (0, 0, 0),
            "no invented issued counters or pending requests"
        );
        if let Some(issuer) = issuer {
            assert!(
                rig.registry()
                    .live_orders_of(match_id, issuer, i64::MAX)
                    .contains(&squad),
                "wall time cannot expire recovered arena watch"
            );
        }
        // Fixture owns this task; release it before the next branch.
        rig.registry()
            .tasks()
            .shutdown(tacsim::arena_bridge::JOIN_TIMEOUT)
            .await;
    }
}

/// Reconstruct the whole production composition. The older AF.3 helper only
/// reset arena/Sim slots and called boot again on the existing BulkService;
/// modern authored-compute hydration correctly refuses that warmed service.
async fn reboot_fresh(rig: &mut Rig, cfg: TacsimConfig) {
    rig.registry()
        .tasks()
        .shutdown(tacsim::arena_bridge::JOIN_TIMEOUT)
        .await;
    if let Some(rx) = rig.egress.as_mut() {
        while rx.try_recv().is_ok() {}
    }
    let _ = rig.pump_ops();
    let data = rig
        .image
        .iter()
        .map(|(ns, docs)| {
            let mut data = mush_core::ModuleBootData {
                kv: docs
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                ..Default::default()
            };
            data.normalize();
            (ns.clone(), data)
        })
        .collect();
    let old = std::mem::replace(&mut rig.tw, TestWorld::new());
    rig.tw = old
        .reboot_into(roster_with(cfg), data)
        .expect("actual fresh production boot");
    let mut accounts = Accounts::default();
    for (n, pilot) in rig.pilots.iter().enumerate() {
        accounts.0.insert(
            *pilot,
            AccountId::mint(&format!("account-{n}"), n as i64).expect("actual fixture account"),
        );
    }
    *rig.tw
        .world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("actual Sim account owner") = mush_sim::AccountOracle::new(Arc::new(accounts));
    for pilot in &rig.pilots {
        rig.tw.connect(*pilot);
    }
    rig.egress = rig.registry().tasks().take_egress();
    assert!(
        rig.egress.is_some(),
        "new process owns a fresh egress receiver"
    );
    let _ = rig.pump_ops();
}
