//! Reuses the parent's actual second certificate. No extra worker run, forged
//! rating, status mutation, source capability or readiness bit is introduced.
use super::*;
use mush_market::mind_trade::records::MindListing;
use mush_org::OrgStore;
use mush_roll::commerce::{FiledRevisionId, registry_recall::RegistryRecallPolicy};

const START: u128 = 56_000_000;
const LISTINGS: &str = "market_mind_original_listing";

fn boot(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: Option<RegistryRecallPolicy>,
) -> TestResult<PlayerWorld> {
    PlayerWorld::boot_options_with_recall(
        f, saved, false, false, false, false, false, true, false, true, false, false, false, policy,
    )
}
fn action(w: &PlayerWorld, owner: &str, id: &str) -> TestResult<DefinitionRef> {
    w.engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|e| e.reference())
        .find(|r| {
            r.key().owner.as_str() == owner
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == 1
        })
        .cloned()
        .ok_or_else(|| "actual Follow action".into())
}
fn signed(
    w: &mut PlayerWorld,
    owner: &str,
    id: &str,
    actor: ObjId,
    n: u128,
    params: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x6f; 32])?,
        [],
    )?);
    let action = action(w, owner, id)?;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&action)
        .ok_or("actual signed Follow action")?;
    let invocation = request(n, actor, owner, params.clone())?;
    let resolved = w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: invocation.request(),
            principal,
            actor,
            idempotency: invocation.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: params,
        },
    )?;
    let (actual, resolved, _) = resolved.into_authoritative_parts();
    assert_eq!(actual, action);
    Ok(w.engine
        .prepare_registered_composed_resolved_authoritative_action(&action, &resolved, token(n))?)
}
fn install(w: &mut PlayerWorld, p: PreparedComposedTxn) -> TestResult {
    let (writes, installation) = p.into_parts();
    captured::apply(w, &writes, installation)
}
fn listing(w: &mut PlayerWorld, n: u128, expected: &FiledRecord) -> TestResult {
    let (p, _) = signed(
        w,
        "market",
        "list_original_mind",
        GOD,
        n,
        json!({"owner_licence":"1"}),
    )?;
    install(w, p)?;
    let row: MindListing = serde_json::from_value(
        rows(&disk(w), "market", LISTINGS)
            .into_iter()
            .find(|v| v.get("publication") == Some(&json!(token(n))))
            .ok_or("actual listing")?,
    )?;
    assert_eq!(row.subject().filed(), expected.content().id());
    assert_eq!(row.subject().revision(), expected.content().revision());
    Ok(())
}
fn unavailable(w: &mut PlayerWorld, n: u128) -> TestResult {
    let before = disk(w);
    assert!(
        signed(
            w,
            "market",
            "list_original_mind",
            GOD,
            n,
            json!({"owner_licence":"1"})
        )
        .is_err()
    );
    assert!(disk(w) == before, "refused Follow changed durable records");
    let events = w.say(GOD, "SIM MIND /filed")?;
    assert!(!PlayerWorld::private_text(&events, GOD).contains("filed:1:"));
    Ok(())
}
fn withdraw(w: &mut PlayerWorld, n: u128, filed: FiledRevisionId) -> TestResult {
    let (p, _) = signed(
        w,
        "roll",
        "withdraw_authored_mind",
        GOD,
        n,
        json!({"filed":filed.get().to_string()}),
    )?;
    install(w, p)
}
fn stage_listing(
    w: &mut PlayerWorld,
    n: u128,
    p: PreparedComposedTxn,
    o: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let action = action(w, "market", "list_original_mind")?;
    let invocation = request(n, GOD, "market", json!({"owner_licence":"1"}))?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        o.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(n),
        request: invocation.request(),
        idempotency: invocation.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([71; 32]),
        request_hash: invocation.request_hash(),
        plan_hash: CanonicalHash::from_bytes([72; 32]),
        action,
        provider: OwnerId::new("market")?,
        cause: mush_core::action::PreparedCausalRef::new(invocation.request(), None, 0)?,
        definitions,
        writes: p.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(n))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = w
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    w.engine.stage_authoritative_action(p, guards, bundle)?;
    Ok(())
}

pub(super) async fn exercise(
    f: &PurchaseFixture,
    certified: &Durable,
    revision: &Revision,
    rating: RatingRef,
) -> TestResult {
    let mut w = boot(f, certified, None)?;
    let mut tasks = vec![w.tasks.clone()];
    let result: TestResult = async {
        let old: FiledRecord =
            serde_json::from_value(one(rows(&disk(&w), "roll", "roll_mind_filed"))?)?;
        let old_terms = rows(&disk(&w), "roll", "roll_mind_owner_term");
        let old_paid = rows(&disk(&w), "roll", "roll_mind_paid_term");
        listing(&mut w, START, &old)?;
        // Actual later IC time at publication gives a real before/exact boundary.
        w.engine
            .world
            .set_now(w.engine.world.now().checked_add(60).ok_or("IC boundary")?);
        let (p, _) = signed(
            &mut w,
            "roll",
            "update_original_mind",
            GOD,
            START + 1,
            parameters(revision, rating),
        )?;
        install(&mut w, p)?;
        let new: FiledRecord = serde_json::from_value(
            rows(&disk(&w), "roll", "roll_mind_filed")
                .into_iter()
                .find(|v| v.get("format") == Some(&json!(3)))
                .ok_or("actual update")?,
        )?;
        assert!(new.content().filed_at().get() > old.content().filed_at().get());
        listing(&mut w, START + 2, &new)?;
        let updated = Durable::capture(&w.engine, &disk(&w));
        // Root installation and the retained owner-read observation both matter:
        // a status change after preparation cannot publish an obsolete selection.
        for after_stage in [false, true] {
            let mut fence = boot(f, &updated, None)?;
            tasks.push(fence.tasks.clone());
            let n = START + 10 + u128::from(after_stage);
            let (p, o) = signed(
                &mut fence,
                "market",
                "list_original_mind",
                GOD,
                n,
                json!({"owner_licence":"1"}),
            )?;
            if after_stage {
                stage_listing(&mut fence, n, p, o)?;
                withdraw(
                    &mut fence,
                    START + 12 + u128::from(after_stage),
                    new.content().id(),
                )?;
                assert!(fence.engine.take_staged_action().is_err());
            } else {
                withdraw(&mut fence, START + 12, new.content().id())?;
                assert!(stage_listing(&mut fence, n, p, o).is_err());
            }
            assert!(
                !rows(&disk(&fence), "market", LISTINGS)
                    .iter()
                    .any(|v| v.get("publication") == Some(&json!(token(n))))
            );
        }
        // Persisted Filed times are absolute IC seconds. The fixture's actual
        // default clock is ratio1/offset0; no new epoch interpretation is added.
        assert_eq!(w.engine.world.world_ic_now().at, w.engine.world.now());
        w.engine.world.set_now(new.content().filed_at().get() - 1);
        listing(&mut w, START + 20, &old)?;
        w.engine.world.set_now(new.content().filed_at().get());
        listing(&mut w, START + 21, &new)?;
        withdraw(&mut w, START + 22, new.content().id())?;
        listing(&mut w, START + 23, &old)?;
        let withdrawn = Durable::capture(&w.engine, &disk(&w));
        let mut cold = boot(f, &withdrawn, None)?;
        tasks.push(cold.tasks.clone());
        listing(&mut cold, START + 24, &old)?;
        assert_eq!(
            rows(&disk(&cold), "roll", "roll_mind_owner_term"),
            old_terms
        );
        assert_eq!(rows(&disk(&cold), "roll", "roll_mind_paid_term"), old_paid);
        withdraw(&mut cold, START + 25, old.content().id())?;
        unavailable(&mut cold, START + 26)?;
        let mut empty = boot(f, &Durable::capture(&cold.engine, &disk(&cold)), None)?;
        tasks.push(empty.tasks.clone());
        unavailable(&mut empty, START + 27)?;
        let paid = empty.say(BUYER, "SIM MIND /filed")?;
        assert!(
            PlayerWorld::private_text(&paid, BUYER).contains("filed:2:"),
            "paid Pin remains usable after author withdrawal"
        );
        // Broken newest history is not an eligibility omission, even after its
        // valid withdrawal. Complete cold reconstruction must refuse it.
        let mut missing = withdrawn.clone();
        assert_eq!(
            remove_family(&mut missing, "roll", "roll_mind_line_update"),
            1
        );
        assert!(boot(f, &missing, None).is_err());

        // A separate real Org-authorized exact-filed recall has the same
        // selection effect, without recalling its shared historical rating.
        let mut authority = boot(f, &updated, None)?;
        tasks.push(authority.tasks.clone());
        authority.say(GOD, "@org/create FollowRegistry")?;
        authority.say(GOD, "@org/scope create FollowRegistry=recall")?;
        let state = authority
            .engine
            .world
            .module_state::<OrgStore>()
            .ok_or("actual Org")?
            .state();
        let org = state
            .orgs()
            .find(|o| o.name.as_str() == "FollowRegistry")
            .ok_or("actual organization")?
            .id;
        let scope = state
            .scopes_in(org)
            .find(|s| s.name.as_str() == "recall")
            .ok_or("actual scope")?
            .id;
        authority.say(
            GOD,
            &format!(
                "@org/grant #{}=FollowRegistry roll.registry.recall scope={}",
                BUYER.0, scope.0
            ),
        )?;
        let policy = RegistryRecallPolicy::new(issuer()?, org, scope, vec![BUYER])?;
        let saved = Durable::capture(&authority.engine, &disk(&authority));
        let mut recalled = boot(f, &saved, Some(policy.clone()))?;
        tasks.push(recalled.tasks.clone());
        let ratings = rows(&disk(&recalled), "roll", "rating_status");
        assert!(!ratings.is_empty(), "actual shared rating statuses");
        let (p, _) = signed(
            &mut recalled,
            "roll",
            "registry_revision_recall",
            BUYER,
            START + 30,
            json!({"filed":new.content().id().get().to_string()}),
        )?;
        install(&mut recalled, p)?;
        listing(&mut recalled, START + 31, &old)?;
        assert_eq!(rows(&disk(&recalled), "roll", "rating_status"), ratings);
        let mut cold = boot(
            f,
            &Durable::capture(&recalled.engine, &disk(&recalled)),
            Some(policy),
        )?;
        tasks.push(cold.tasks.clone());
        listing(&mut cold, START + 32, &old)?;
        Ok(())
    }
    .await;
    for tasks in tasks {
        tasks.shutdown(Duration::from_secs(5)).await;
    }
    result
}
