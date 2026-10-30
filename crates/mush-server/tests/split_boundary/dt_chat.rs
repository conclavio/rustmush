//! DT.5 uses the real roster commit, Comsys clients and Propagation scheduler.
use super::*;
use mush_core::QueryHandler;
use mush_proto::OutputEvent;

fn chat_channel(
    world: &TestWorld,
    organizer: ObjId,
    activation: u64,
) -> TestResult<mush_comsys::ChannelId> {
    mush_comsys::managed::channel(world.world(), &format!("dt:{}:1:{activation}", organizer.0))
        .ok_or_else(|| "roster channel".into())
}
fn alias(world: &TestWorld, actor: ObjId, channel: mush_comsys::ChannelId) -> TestResult<String> {
    world
        .world()
        .module_state::<mush_comsys::ComsysState>()
        .and_then(|s| s.members.get(&(actor, channel)))
        .map(|m| m.alias.to_string())
        .ok_or_else(|| "roster alias".into())
}
fn backlog(world: &TestWorld, actor: ObjId, name: &str) -> TestResult<Value> {
    match mush_comsys::BacklogQuery.query(
        world.world(),
        actor,
        &json!({"channel":name,"limit":200}),
    ) {
        Ok(OutputEvent::Panel { data, .. }) => Ok(data),
        other => Err(format!("backlog query: {other:?}").into()),
    }
}
fn name(world: &TestWorld, channel: mush_comsys::ChannelId) -> TestResult<String> {
    mush_comsys::api::channel_name(world.world(), channel)
        .map(|n| n.to_string())
        .ok_or_else(|| "channel name".into())
}
async fn settle_chat(world: &mut TestWorld, store: &InMemoryStore) -> TestResult {
    for _ in 0..8 {
        pulse(world, store, 2).await?;
    }
    Ok(())
}

#[tokio::test]
async fn dt5_chat_membership_is_the_accepted_roster_and_chat_is_not_an_order() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let (organizer, player, stranger) = (rig.organizer, rig.invitee, rig.stranger);
    let channel = chat_channel(&rig.world, organizer, 1)?;
    let name = name(&rig.world, channel)?;
    assert_eq!(
        mush_comsys::api::members_of(rig.world.world(), channel),
        vec![player]
    );
    let shown = text(&mut rig.world, player, "+sortie/chat");
    assert!(
        shown.contains("IC transit") && shown.contains("dt <message>"),
        "{shown}"
    );
    for actor in [organizer, stranger] {
        let refused = text(
            &mut rig.world,
            actor,
            &format!("+sortie/chat {}:1:1=outsider", organizer.0),
        );
        assert!(
            refused.contains("No available detachment chat"),
            "{refused}"
        );
        assert!(!text(&mut rig.world, actor, "clist").contains(&name));
        assert!(
            !text(&mut rig.world, actor, &format!("@channel/who {name}"))
                .contains("Planning invitee")
        );
        assert!(
            !text(&mut rig.world, actor, &format!("addcom stolen={name}"))
                .contains("added with alias")
        );
        assert!(backlog(&rig.world, actor, &name).is_err());
    }
    // Even ordinary channel administration cannot turn this IC net into a bypass.
    let altered = text(&mut rig.world, GOD, &format!("@channel/set {name}=!ic"));
    assert!(altered.contains("managed by its roster"), "{altered}");
    rig.world.connect(player);
    let before = rig
        .world
        .engine()
        .action_state()
        .get(
            &rig.world
                .world()
                .module_state::<tacsim::detachment::activation::ActivationHandles>()
                .ok_or("handles")?
                .records,
            &tacsim::detachment::planning::DraftRef {
                organizer,
                id: PlanId(1),
            },
        )?
        .cloned();
    let queued = text(
        &mut rig.world,
        player,
        &format!("+sortie/chat {}:1:1=HOLD here = chat text", organizer.0),
    );
    assert!(queued.contains("queued for IC transit"), "{queued}");
    assert!(
        !backlog(&rig.world, player, &name)?
            .to_string()
            .contains("HOLD here")
    );
    persist_legacy(&mut rig.world, &store).await?;
    settle_chat(&mut rig.world, &store).await?;
    let received = backlog(&rig.world, player, &name)?;
    assert!(
        received.to_string().contains("HOLD here = chat text"),
        "{received}"
    );
    let after = rig
        .world
        .engine()
        .action_state()
        .get(
            &rig.world
                .world()
                .module_state::<tacsim::detachment::activation::ActivationHandles>()
                .ok_or("handles")?
                .records,
            &tacsim::detachment::planning::DraftRef {
                organizer,
                id: PlanId(1),
            },
        )?
        .cloned();
    assert_eq!(
        before, after,
        "chat cannot edit the roster or command authority"
    );
    let who = text(&mut rig.world, player, &format!("@channel/who {name}"));
    assert!(
        !who.contains("[connected]") && !who.contains("[gagged]"),
        "{who}"
    );
    assert!(
        !text(
            &mut rig.world,
            player,
            &format!("+sortie/chat {}:1:2=stale", organizer.0)
        )
        .contains("queued for IC transit")
    );
    assert!(
        !text(
            &mut rig.world,
            player,
            &format!("+sortie/chat {}:1:1=", organizer.0)
        )
        .contains("queued for IC transit")
    );
    Ok(())
}

#[tokio::test]
async fn dt5_chat_jamming_and_restart_preserve_only_each_recipients_delivered_lines() -> TestResult
{
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_field_fixture(&store, true).await?;
    let (captain, npc) = (rig.organizer, rig.invitee);
    let channel = chat_channel(&rig.world, captain, 1)?;
    let name = name(&rig.world, channel)?;
    let captain_alias = alias(&rig.world, captain, channel)?;
    let npc_alias = alias(&rig.world, npc, channel)?;
    let npc_place = mush_propagation::place_of(
        rig.world
            .world()
            .module_state::<mush_sim::SpatialState>()
            .ok_or("spatial")?,
        rig.world.world(),
        npc,
    )
    .ok_or("NPC radio endpoint")?;
    rig.world.connect(captain);
    ordinary(
        &mut rig.world,
        GOD,
        &format!(
            "@prop/jam #{}={}",
            npc_place.0,
            mush_propagation::JAMMING_BLOCKS_AT
        ),
    )?;
    let queued = text(
        &mut rig.world,
        captain,
        &format!("{captain_alias} Waiting beside the ramp."),
    );
    assert!(queued.contains("queued for IC transit"), "{queued}");
    persist_legacy(&mut rig.world, &store).await?;
    settle_chat(&mut rig.world, &store).await?;
    assert!(
        backlog(&rig.world, captain, &name)?
            .to_string()
            .contains("Waiting beside the ramp.")
    );
    assert!(
        !backlog(&rig.world, npc, &name)?
            .to_string()
            .contains("Waiting beside the ramp."),
        "jammed membership cannot hydrate unseen source history"
    );
    let now = rig.world.world().now();
    drop(rig);
    let mut restarted = fixture_keyed(&store, true).await?;
    restarted.world.set_now(now);
    assert_eq!(chat_channel(&restarted.world, captain, 1)?, channel);
    assert!(
        !backlog(&restarted.world, npc, &name)?
            .to_string()
            .contains("Waiting beside the ramp.")
    );
    ordinary(
        &mut restarted.world,
        GOD,
        &format!("@prop/jam #{}=0", npc_place.0),
    )?;
    persist_legacy(&mut restarted.world, &store).await?;
    settle_chat(&mut restarted.world, &store).await?;
    let arrived = backlog(&restarted.world, npc, &name)?;
    assert_eq!(
        arrived
            .to_string()
            .matches("Waiting beside the ramp.")
            .count(),
        1,
        "{arrived}"
    );
    let captain_place = mush_propagation::place_of(
        restarted
            .world
            .world()
            .module_state::<mush_sim::SpatialState>()
            .ok_or("spatial")?,
        restarted.world.world(),
        captain,
    )
    .ok_or("captain radio endpoint")?;
    ordinary(
        &mut restarted.world,
        GOD,
        &format!(
            "@prop/jam #{}={}",
            captain_place.0,
            mush_propagation::JAMMING_BLOCKS_AT
        ),
    )?;
    // A normal reply uses exactly the same real return path.
    let reply = text(
        &mut restarted.world,
        npc,
        &format!("{npc_alias} :signals ready."),
    );
    assert!(reply.contains("queued for IC transit"), "{reply}");
    persist_legacy(&mut restarted.world, &store).await?;
    assert!(
        !backlog(&restarted.world, captain, &name)?
            .to_string()
            .contains("signals ready.")
    );
    settle_chat(&mut restarted.world, &store).await?;
    assert!(
        !backlog(&restarted.world, captain, &name)?
            .to_string()
            .contains("signals ready.")
    );
    assert!(
        backlog(&restarted.world, npc, &name)?
            .to_string()
            .contains("signals ready.")
    );
    ordinary(
        &mut restarted.world,
        GOD,
        &format!("@prop/jam #{}=0", captain_place.0),
    )?;
    persist_legacy(&mut restarted.world, &store).await?;
    settle_chat(&mut restarted.world, &store).await?;
    let received = backlog(&restarted.world, captain, &name)?;
    assert!(
        received.to_string().contains("signals ready."),
        "{received}"
    );
    settle_chat(&mut restarted.world, &store).await?;
    assert_eq!(
        backlog(&restarted.world, npc, &name)?
            .to_string()
            .matches("Waiting beside the ramp.")
            .count(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn dt5_chat_closure_commits_with_roster_and_new_activation_has_no_old_traffic() -> TestResult
{
    let store = InMemoryStore::new();
    let (mut rig, cab) = field_fixture(&store).await?;
    let (organizer, player) = (rig.organizer, rig.invitee);
    let old = chat_channel(&rig.world, organizer, 1)?;
    let old_name = name(&rig.world, old)?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!(
            "@prop/jam #{}={}",
            cab.0,
            mush_propagation::JAMMING_BLOCKS_AT
        ),
    )?;
    assert!(text(&mut rig.world, player, "dt old pending line").contains("queued for IC transit"));
    persist_legacy(&mut rig.world, &store).await?;
    let request = submit(&mut rig.world, organizer, "+sortie/deactivate 1:1")?;
    stage(&mut rig.world, &store, &request).await?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged deactivation")?
        .into_parts();
    assert_eq!(
        chat_channel(&rig.world, organizer, 1)?,
        old,
        "preparation grants or revokes nothing"
    );
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    rig.world
        .engine_mut()
        .install_composed_action(installation)?;
    assert!(
        mush_comsys::managed::channel(rig.world.world(), &format!("dt:{}:1:1", organizer.0))
            .is_none()
    );
    assert!(backlog(&rig.world, player, &old_name).is_err());
    assert!(!text(&mut rig.world, player, "dt cannot send now").contains("queued for IC transit"));
    commit(
        &mut rig.world,
        &store,
        organizer,
        "+sortie/activate 1:1=combat",
    )
    .await?;
    let new = chat_channel(&rig.world, organizer, 2)?;
    assert_ne!(new, old);
    assert_eq!(
        alias(&rig.world, player, new)?,
        "dt",
        "archived membership does not consume the active alias"
    );
    ordinary(&mut rig.world, GOD, &format!("@prop/jam #{}=0", cab.0))?;
    persist_legacy(&mut rig.world, &store).await?;
    settle_chat(&mut rig.world, &store).await?;
    let new_name = name(&rig.world, new)?;
    assert!(
        !backlog(&rig.world, player, &new_name)?
            .to_string()
            .contains("old pending line")
    );
    assert!(text(&mut rig.world, player, "dt new roster line").contains("queued for IC transit"));
    persist_legacy(&mut rig.world, &store).await?;
    settle_chat(&mut rig.world, &store).await?;
    assert!(
        backlog(&rig.world, player, &new_name)?
            .to_string()
            .contains("new roster line")
    );
    Ok(())
}

#[tokio::test]
async fn dt5_chat_member_can_rejoin_without_replaying_traffic_or_revising_the_roster() -> TestResult
{
    let store = InMemoryStore::new();
    let (mut rig, cab) = field_fixture(&store).await?;
    let (organizer, player) = (rig.organizer, rig.invitee);
    let channel = chat_channel(&rig.world, organizer, 1)?;
    let name = name(&rig.world, channel)?;
    ordinary(&mut rig.world, player, "dt before leaving")?;
    persist_legacy(&mut rig.world, &store).await?;
    settle_chat(&mut rig.world, &store).await?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!(
            "@prop/jam #{}={}",
            cab.0,
            mush_propagation::JAMMING_BLOCKS_AT
        ),
    )?;
    ordinary(&mut rig.world, player, "dt pending before rejoining")?;
    ordinary(&mut rig.world, player, "delcom dt")?;
    persist_legacy(&mut rig.world, &store).await?;
    assert!(backlog(&rig.world, player, &name).is_err());
    commit(
        &mut rig.world,
        &store,
        player,
        &format!("+sortie/chat {}:1:1", organizer.0),
    )
    .await?;
    assert_eq!(alias(&rig.world, player, channel)?, "dt");
    assert!(
        !backlog(&rig.world, player, &name)?
            .to_string()
            .contains("before leaving")
    );
    ordinary(&mut rig.world, GOD, &format!("@prop/jam #{}=0", cab.0))?;
    assert!(text(&mut rig.world, player, "+sortie/chat").contains("dt <message>"));
    assert!(text(&mut rig.world, player, "dt after rejoining").contains("queued for IC transit"));
    persist_legacy(&mut rig.world, &store).await?;
    settle_chat(&mut rig.world, &store).await?;
    assert!(
        backlog(&rig.world, player, &name)?
            .to_string()
            .contains("after rejoining")
    );
    assert!(
        !backlog(&rig.world, player, &name)?
            .to_string()
            .contains("pending before rejoining")
    );
    Ok(())
}

#[tokio::test]
async fn dt5_chat_initialises_an_older_accepted_roster_after_restart_without_new_consent()
-> TestResult {
    use mush_store::ObjectStore;
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let (organizer, player) = (rig.organizer, rig.invitee);
    let old = chat_channel(&rig.world, organizer, 1)?;
    persist_legacy(&mut rig.world, &store).await?;
    // Simulate the pre-DT.5 durable world: the accepted source roster exists,
    // but no channel or channel membership has been authored yet.
    store
        .write(mush_store::WriteBatch {
            serial: 1,
            module_ops: vec![
                mush_core::ModuleOp {
                    ns: mush_comsys::NS.into(),
                    op: mush_core::ModuleWrite::Del {
                        key: mush_comsys::chan_key(old),
                    },
                },
                mush_core::ModuleOp {
                    ns: mush_comsys::NS.into(),
                    op: mush_core::ModuleWrite::Del {
                        key: mush_comsys::member_key(player, old),
                    },
                },
            ],
            ..Default::default()
        })
        .await?;
    drop(rig);
    let mut restarted = fixture(&store).await?;
    assert!(chat_channel(&restarted.world, organizer, 1).is_err());
    commit(
        &mut restarted.world,
        &store,
        player,
        &format!("+sortie/chat {}:1:1", organizer.0),
    )
    .await?;
    let channel = chat_channel(&restarted.world, organizer, 1)?;
    assert_ne!(channel, old);
    assert_eq!(
        mush_comsys::api::members_of(restarted.world.world(), channel),
        vec![player]
    );
    assert!(text(&mut restarted.world, player, "+sortie/chat").contains("dt <message>"));
    Ok(())
}
