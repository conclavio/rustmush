#[tokio::test]
async fn retro_private_access_disclosure_and_size() {
    let mut rig = Rig::new(true).await;
    let bob = rig.endpoints();
    let operator = rig.registry().operator;
    // Normal economic title identifies an org-owned receiving unit, not a payer.
    rig.run(GOD, "@create PolicyReceiverUnit");
    let unit = rig
        .tw
        .world()
        .iter()
        .find(|o| o.name == "PolicyReceiverUnit")
        .unwrap()
        .id;
    let ceres = rig.registry().nodes[&cid("ceres")].place;
    rig.run(GOD, &format!("@teleport {unit}={ceres}"));
    let title = rig.run(GOD, &format!("@asset/title {unit}=org:{operator}"));
    assert!(title.contains("Title recorded"), "{title}");
    rig.run(
        GOD,
        &format!(
            "+relay/policy luna_ceres/org:{operator}=suspended,operator.org,organization denied"
        ),
    );
    rig.run(GOD, &format!("+order {unit}=organization receiver"));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    let npc = rig
        .tw
        .world_mut()
        .create(mush_core::ObjKind::Npc, "Policy NPC", GOD, Some(ceres))
        .unwrap();
    assert!(
        rig.run(
            GOD,
            &format!(
                "+relay/policy luna_ceres/{npc}=suspended,operator.npc,Personal NPC restriction"
            )
        )
        .contains("policy recorded")
    );
    rig.run(GOD, &format!("+order {npc}=npc receiver"));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    // Personal membership/physical location cannot accidentally become org identity.
    rig.run(GOD, &format!("+order {bob}=personal receiver"));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .any(|l| l.pair.is_some())
    );
    rig.run(
        GOD,
        &format!("+relay/policy luna_ceres/{bob}=suspended,operator.person,personal denial"),
    );
    rig.run(GOD, &format!("+order {bob}=denied on first operator"));
    assert!(
        rig.last_envelope()
            .transit()
            .unwrap()
            .legs
            .iter()
            .all(|l| l.pair.is_none())
    );
    // Controlled persisted topology fixture with a second, independently owned
    // pair. Objects/behavior bindings use normal APIs; only the topology document
    // is fixture input. R9 supplies manufacture/deployment, not this test fixture.
    let other = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .orgs()
        .find(|o| o.id != operator)
        .unwrap()
        .id;
    let mut catalog = rig.registry();
    let mut pair = catalog.pairs[&cid("luna_ceres")].clone();
    for (i, key) in ["malapert", "ceres"].iter().enumerate() {
        let mut node = catalog.nodes[&cid(key)].clone();
        let node_name = format!("R4IndependentNode{i}");
        rig.run(GOD, &format!("@create {node_name}"));
        node.object = rig
            .tw
            .world()
            .iter()
            .find(|o| o.name == node_name)
            .unwrap()
            .id;
        rig.run(GOD, &format!("@teleport {}={}", node.object, node.place));
        rig.run(
            GOD,
            &format!("@behavior/add {}=sim.relay_node", node.object),
        );
        rig.run(
            GOD,
            &format!("@unit/create {}=tacsim.unit.relay_station", node.object),
        );
        let half_name = format!("R4IndependentHalf{i}");
        let half = rig
            .tw
            .world_mut()
            .create(
                mush_core::ObjKind::Thing,
                &half_name,
                GOD,
                Some(node.object),
            )
            .unwrap();
        rig.run(GOD, &format!("@behavior/add {half}=sim.relay_half"));
        node.operator = other;
        let key = cid(&format!("independent_{key}"));
        pair.halves[i].node = key.clone();
        pair.halves[i].object = half;
        pair.halves[i].id = cid(&format!("independent.half{i}"));
        catalog.nodes.insert(key, node);
    }
    pair.class_policy = mush_sim::relay::ClassPolicy::Private;
    pair.capacity_units_per_ic_second = 1;
    catalog.pairs.insert(cid("independent"), pair);
    catalog.revision += 1;
    catalog.reconcile(rig.tw.world()).unwrap();
    rig.flush().await;
    rig.replace_document(serde_json::to_value(&catalog).unwrap())
        .await;
    rig.restart().await.unwrap();
    assert!(
        rig.run(GOD, "@atlas/pack plan tacsim.relay.luna_ceres")
            .contains("Nothing to do.")
    );

    let outsider=rig.tw.spawn_player("RetroOutsider");
    let visible=rig.run(outsider,"+relay/status independent");
    let absent=rig.run(outsider,"+relay/status independent_missing");
    assert!(visible.contains("Served"),"{visible}");
    assert!(absent.contains("No such public link"),"{absent}");
    let api=rig.tw.world().module_state::<mush_propagation::routing::RoutingServices>().unwrap().1.as_ref().unwrap();
    assert!(!api.authorized(rig.tw.world(),outsider,&cid("independent"),"relay.inspect"));
    let w=rig.tw.world();let at=w.world_ic_now();
    let services=w.module_state::<mush_propagation::routing::RoutingServices>().unwrap();
    let metric=services.0.snapshot(w,&at);
    let spatial=w.module_state::<mush_sim::SpatialState>().unwrap();
    let r=rig.registry();
    let from=r.nodes[&cid("malapert")].place;let to=r.nodes[&cid("ceres")].place;
    let plan=|units|mush_propagation::routing::plan(w,spatial,metric.as_ref(),from,to,&at,0,units,Some((outsider,bob))).0.unwrap();
    assert!(plan(1).legs.iter().any(|l|l.pair==Some(cid("independent"))));
    assert!(plan(2).legs.iter().all(|l|l.pair.is_none()));
    println!("REVIEW: unprivileged outsider status={visible:?}; unknown={absent:?}; one-unit traffic uses private pair without grant; two-unit traffic bypasses private pair.");
}

#[tokio::test]
async fn retro_two_spoke_standard_waits_twice(){
    let mut rig=Rig::new(true).await;let bob=rig.endpoints();
    let r=rig.registry();let from=r.nodes[&cid("ceres")].place;let to=r.nodes[&cid("mercury")].place;
    rig.run(GOD,&format!("@teleport #1={from}"));rig.run(GOD,&format!("@teleport {bob}={to}"));
    for pair in ["luna_ceres","luna_mercury"] {assert!(rig.run(GOD,&format!("+relay/grant {pair}/#1=standard")).contains("Class grant recorded"));}
    rig.run(GOD,&format!("+order {bob}=two-spoke-standard"));let id=rig.last_envelope().id();
    let start=rig.tw.world().world_ic_now().at;
    for t in 0..=130 {rig.advance_ic_to(start+t);rig.pass();}
    assert_eq!(rig.envelope(id).state(),mush_propagation::EnvelopeState::Delivered);
    let receipts:Vec<_>=rig.tw.world().module_state::<mush_propagation::PropagationStore>().unwrap().relay_transport().service.as_ref().unwrap().receipts.iter().filter(|r|r.envelope==id).collect();
    assert_eq!(receipts.len(),2);
    assert!(receipts.iter().all(|r|r.transmitted-r.queued>=60));
    println!("REVIEW: Standard two-spoke waits {:?}",receipts.iter().map(|r|(&r.pair,r.queued-start,r.transmitted-start)).collect::<Vec<_>>());
}

#[tokio::test]
async fn retro_mail_reservations_block_other_producers(){
    let mut rig=Rig::new(true).await;let bob=rig.endpoints();let sender=rig.tw.spawn_player("RetroSender");let a=rig.tw.spawn_player("RetroA");let b=rig.tw.spawn_player("RetroB");
    for n in 0..64 {let reply=rig.run(sender,&format!("@mail/quick {a} {b}=fill-{n}/queued"));assert!(reply.contains("queued for IC transit"),"{n}: {reply}");}
    for _ in 0..10 {rig.mail_tick();}
    let reply=rig.run(GOD,&format!("+order {bob}=critical-order"));
    assert!(reply.to_lowercase().contains("full"),"{reply}");
    println!("REVIEW: one sender's 128 reserved mail copies block unrelated orders: {reply}");
}
