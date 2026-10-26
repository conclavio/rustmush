#![cfg(test)]

//! Actual Registry -> paid Grant -> fork -> worker Save supplies these branches.
//! Complete owner+typed union and cold boot are tested; server writer transport
//! and ordinary @mind/remove UX remain separate, explicitly unclaimed work.
use super::*;
#[path = "authored_retirement_entry_tests.rs"]
mod entry_tests;
fn action(world: &PlayerWorld) -> TestResult<DefinitionRef> {
    world
        .engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|e| e.reference())
        .find(|r| {
            r.key().owner.as_str() == "roll"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == "retire_authored_draft"
                && r.version() == 1
        })
        .cloned()
        .ok_or_else(|| "retirement action".into())
}
fn parameters(mind: &mush_roll::Mind) -> Value {
    json!({"name":mind.name(),"edit":mind.base().edit.to_string(),"revision":mind.base().revision.map(|n|n.to_string()).unwrap_or_else(||"none".into())})
}
fn prepare(
    world: &mut PlayerWorld,
    actor: ObjId,
    number: u128,
    parameters: Value,
) -> TestResult<PreparedComposedTxn> {
    let action = action(world)?;
    Ok(world
        .engine
        .prepare_registered_composed_authoritative_action(
            &action,
            &crate::minds::bulk::registry_integration_tests::original_purchase::request(
                number, actor, "roll", parameters,
            )?,
            token(number),
        )?
        .0)
}
fn publish(world: &mut PlayerWorld, prepared: PreparedComposedTxn) -> TestResult<Disk> {
    let (writes, installation) = prepared.into_parts();
    let mut retained = disk(world);
    retain_writes(&mut retained, &writes)?;
    let ops = writes
        .into_iter()
        .map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => Ok(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: key.to_string().into(),
                    value,
                },
            }),
            ReplayWrite::ModuleDelete { namespace, key } => Ok(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::Del {
                    key: key.to_string().into(),
                },
            }),
            _ => Err("retirement left Roll metadata scope"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    mush_testkit::fold_module_ops_into(&mut world.image, ops);
    world.engine.install_composed_action(installation)?;
    world.commit()?;
    Ok(retained)
}
fn unchanged_business(before: &Disk, after: &Disk) {
    let business = |disk: &Disk| {
        disk.iter()
            .filter(|((owner, _), row)| {
                owner == "economy"
                    || owner == "market"
                    || (owner == "roll"
                        && row
                            .pointer("/family/id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| {
                                !matches!(
                                    id,
                                    "roll_mind_authored_retirement"
                                        | "roll_mind_retired_draft_parent"
                                )
                            }))
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Disk>()
    };
    assert!(
        business(before) == business(after),
        "retirement cannot change money, grants, selectors or filings"
    );
}
fn no_source(events: &Events) {
    assert!(
        events
            .iter()
            .all(|(_, e)| !serde_json::to_string(e).is_ok_and(|text| text.contains("this.landed"))),
        "metadata cleanup output has no source"
    );
}
async fn success(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result:TestResult=async {
        // A real worker creates a named immutable revision; unnamed saves already exist.
        let base=roll(&world)?.mind(BUYER,"child")?.base();
        let job=match invoke(&mut world,BUYER_CONN,REQUEST,MindRequest::Save {name:"child".into(),base:MindEditBase {edit:base.edit,revision:base.revision},notes:"retained cleanup inventory".into(),version:Some("retained".into())})? {
            MindResponse::Queued {job}=>job,_=>return Err("actual named save not queued".into()),
        };
        ready(&world,job).await?;assert!(matches!(terminal(&drain(&mut world,job).await?)?,MindResponse::Saved {..}));
        let child=roll(&world)?.mind(BUYER,"child")?.clone();let params=parameters(&child);
        let term:PaidTermRecord=serde_json::from_value(one(rows(&disk(&world),"roll","roll_mind_paid_term"))?)?;
        world.engine.world.set_now(term.window().grace_end().get());
        // Lapsed source is still private; metadata cleanup has its own authority.
        assert!(matches!(invoke(&mut world,BUYER_CONN,REQUEST+70,MindRequest::Read {name:"child".into(),revision:None})?,MindResponse::Error {..}));
        let before=disk(&world);let selector=one(rows(&before,"roll","roll_mind_draft_parent"))?;
        for (actor,request) in [(GOD,params.clone()),(BUYER,json!({"name":"child","edit":"0","revision":"none"})),
            (BUYER,json!({"name":"child","edit":child.base().edit.to_string(),"revision":child.base().revision.map(|n|n.to_string()),"actor":GOD}))] {
            assert!(prepare(&mut world,actor,9_400_001,request).is_err());assert!(disk(&world)==before);
        }
        let events=line(&mut world,"@mind/remove child")?;no_source(&events);
        let held=world.engine.take_observed_authoritative_submit().ok_or("captured removal request")?;
        assert!(held.is_observed()); // This branch tests v1 directly; entry transport has its own child.
        drop(held);
        assert!(roll(&world)?.mind(BUYER,"child").is_ok(),"captured removal does not optimistically mutate Roll");
        let prepared=prepare(&mut world,BUYER,9_400_002,params.clone())?;
        assert!(disk(&world)==before && roll(&world)?.mind(BUYER,"child").is_ok(),"no pre-ACK cleanup");
        let writes=prepared.authoritative_replay_writes();let mut committed=before.clone();retain_writes(&mut committed,&writes)?;
        let cold_saved=capture(&world,&committed);
        let cold=PlayerWorld::boot_forks(f,&cold_saved,true)?;
        assert!(roll(&cold)?.mind(BUYER,"child").is_err(),"durable complete union recovers without rerunning current rights");
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        let retained=publish(&mut world,prepared)?;assert!(retained==committed);
        assert!(roll(&world)?.names(BUYER).is_empty(),"last library slot may be removed");
        let tombstone=one(rows(&retained,"roll","roll_mind_retired_draft_parent"))?;
        let op=one(rows(&retained,"roll","roll_mind_authored_retirement"))?;
        assert_eq!(op.get("operation"),Some(&json!(token(9_400_002))));
        assert_eq!(op.get("operation"),tombstone.get("operation"));
        assert_eq!(op.get("selection"),tombstone.get("selection"));
        assert_eq!(op.get("named").and_then(Value::as_array).map(Vec::len),Some(1));
        assert!(op.get("unnamed").and_then(Value::as_array).is_some_and(|v|!v.is_empty()));
        assert!(op.get("source").is_none());
        assert!(one(rows(&retained,"roll","roll_mind_draft_parent"))?==selector);
        unchanged_business(&before,&retained);
        assert!(prepare(&mut world,BUYER,9_400_003,params).is_err());
        assert!(matches!(invoke(&mut world,BUYER_CONN,REQUEST+70,MindRequest::Read {name:"child".into(),revision:Some("1".into())})?,MindResponse::Error {..}));
        // Actual original command behavior is unchanged and identity allocation is not reused.
        no_source(&line(&mut world,"@mind/new child")?);
        let replacement=roll(&world)?.mind(BUYER,"child")?.clone();assert!(replacement.id()>child.id());replacement.require_original_access()?;
        no_source(&line(&mut world,"@mind/remove child")?);assert!(roll(&world)?.mind(BUYER,"child").is_err());
        let final_image=capture(&world,&disk(&world));let restart=PlayerWorld::boot_forks(f,&final_image,true)?;
        restart.tasks.shutdown(Duration::from_secs(5)).await;
        for family in ["roll_mind_draft_parent","roll_mind_retired_draft_parent","roll_mind_authored_retirement"] {
            let mut corrupt=cold_saved.clone();remove_family(&mut corrupt,"roll",family);
            assert!(crate::test_checks::is_refusal(PlayerWorld::boot_forks(f,&corrupt,true)),"missing paired retirement family refuses {family}");
        }
        for prefix in ["reference:roll:commercial:v1/draft_retirement/","reference:roll:unnamed_history:","reference:roll:named_version:"] {
            let mut corrupt=cold_saved.clone();let removed=corrupt.disk.keys().filter(|(ns,k)|ns=="roll" && k.starts_with(prefix)).cloned().collect::<Vec<_>>();
            assert!(!removed.is_empty(),"actual retention fixture exists");for key in removed {corrupt.disk.remove(&key);}
            assert!(crate::test_checks::is_refusal(PlayerWorld::boot_forks(f,&corrupt,true)),"missing actual retirement retention refuses");
        }
        let mut both=cold_saved.clone();
        for ((ns,key),value) in &before {if ns=="roll" && (key==&format!("mind:{}:{}",BUYER.0,child.id()) || key==&format!("library:{}",BUYER.0)) {both.disk.insert((ns.clone(),key.clone()),value.clone());}}
        assert!(crate::test_checks::is_refusal(PlayerWorld::boot_forks(f,&both,true)),"live and retired identity cannot coexist");
        Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
async fn stale(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let child = roll(&world)?.mind(BUYER, "child")?.clone();
        let prepared = prepare(&mut world, BUYER, 9_400_010, parameters(&child))?;
        no_source(&line(
            &mut world,
            "@mind/insert child 1=// actual later owner edit",
        )?);
        let changed = roll(&world)?.mind(BUYER, "child")?.clone();
        assert!(changed.base() != child.base());
        let before = disk(&world);
        let (_, installation) = prepared.into_parts();
        assert!(
            world.engine.install_composed_action(installation).is_err(),
            "captured owner generation cannot erase a later edit"
        );
        assert!(
            disk(&world) == before && roll(&world)?.mind(BUYER, "child")?.base() == changed.base()
        );
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
async fn unsaved_and_queued(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_forks(f, saved, true)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        let (fork, _) = super::super::prepare(
            &mut world,
            BUYER,
            9_400_020,
            json!({"parent_licence":"2","name":"cleanup"}),
        )?;
        publish(&mut world, fork)?;
        let mind = roll(&world)?.mind(BUYER, "cleanup")?.clone();
        assert_eq!(
            mind.base(),
            mush_roll::EditBase {
                edit: 0,
                revision: None
            }
        );
        let job = match invoke(
            &mut world,
            BUYER_CONN,
            REQUEST,
            MindRequest::Save {
                name: "cleanup".into(),
                base: MindEditBase {
                    edit: 0,
                    revision: None,
                },
                notes: "held during metadata cleanup".into(),
                version: None,
            },
        )? {
            MindResponse::Queued { job } => job,
            _ => return Err("actual cleanup save was not queued".into()),
        };
        ready(&world, job).await?;
        let before = disk(&world);
        let prepared = prepare(&mut world, BUYER, 9_400_021, parameters(&mind))?;
        let retained = publish(&mut world, prepared)?;
        unchanged_business(&before, &retained);
        let op = one(rows(&retained, "roll", "roll_mind_authored_retirement"))?;
        for key in ["unnamed", "named"] {
            assert!(
                op.get(key)
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
            );
        }
        assert!(
            roll(&world)?.mind(BUYER, "child").is_ok(),
            "retirement selects one exact owned draft"
        );
        let events = drain(&mut world, job).await?;
        no_source(&events);
        assert!(
            matches!(terminal(&events)?, MindResponse::Error { .. }),
            "ready worker cannot recreate a retired draft"
        );
        let after = capture(&world, &disk(&world));
        let cold = PlayerWorld::boot_forks(f, &after, true)?;
        assert!(roll(&cold)?.mind(BUYER, "cleanup").is_err());
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
pub(super) async fn exercise(f: &PurchaseFixture, saved: &Durable) -> TestResult {
    story_phase("retirement.success", || success(f, saved))?.await?;
    story_phase("retirement.unsaved", || unsaved_and_queued(f, saved))?.await?;
    story_phase("retirement.stale", || stale(f, saved))?.await?;
    story_phase("retirement.entries", || entry_tests::exercise(f, saved))?.await
}
