#![cfg(test)]

//! Real typed transactions, registered retirement/query paths and boot index
//! audits. Seeded offer bodies are structural fixtures, never paid authority.
use super::*;
use crate::mind_trade::index;
use mush_core::{Engine, GameModule, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, Outbox};
use std::collections::BTreeMap;

struct CapturingMarket;
impl GameModule for CapturingMarket {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let families = crate::mind_trade::register(reg, &config(), Arc::new(Gate), false)?;
        reg.state(families);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _: ModuleBootData,
    ) -> Result<(), ModError> {
        let families = ctx
            .w
            .module_state::<Families>()
            .ok_or_else(|| ModError::msg("fixture families"))?
            .clone();
        families.audit(ctx)
    }
}
fn rig() -> (Engine, Families) {
    let e = engine_with_market(Box::new(CapturingMarket)).expect("registered real offer providers");
    let f = e
        .world
        .module_state::<Families>()
        .expect("captured owned handles")
        .clone();
    (e, f)
}
fn id(n: u128) -> CommitToken {
    CommitToken::from_bytes(n.to_be_bytes())
}
fn listing_for(n: u128, seller: ObjId) -> MindListing {
    let mut row = listing();
    row.publication = id(n);
    row.subject.seller = seller;
    row.policy = config().policy.digest().expect("actual policy");
    row
}
fn quote_for(n: u128, buyer: ObjId) -> MindQuote {
    let mut row = quote();
    row.publication = id(n);
    row.buyer = buyer;
    row.policy = config().policy.digest().expect("actual policy");
    row
}
fn change(
    e: &mut Engine,
    f: &Families,
    run: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<(), String>,
) -> Result<Vec<ModuleOp>, String> {
    let mut txn = e
        .action_transaction(&f.listings)
        .map_err(|e| e.to_string())?;
    run(&mut txn)?;
    let (ops, install) = txn.prepare().map_err(|e| e.to_string())?.into_parts();
    e.install_prepared_action(install)
        .map_err(|e| e.to_string())?;
    Ok(ops)
}
fn count(e: &Engine, f: &Families) -> index::Counts {
    e.action_state()
        .get(&f.indices.counts, &())
        .expect("count read")
        .cloned()
        .unwrap_or_default()
}
fn boot(e: &mut Engine) -> Result<(), ModError> {
    e.module_loaded(
        crate::NS,
        ModuleBootData::default(),
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )
}

#[test]
fn per_actor_and_global_caps_use_actual_rows_and_refusal_does_not_mutate() {
    let (mut e, f) = rig();
    for n in 0..index::MAX_LISTINGS {
        if n == index::MAX_SELLER_LISTINGS {
            let before = (e.action_state().revision(), count(&e, &f));
            assert!(
                change(&mut e, &f, |t| index::publish_listing(
                    t,
                    &f,
                    listing_for(9999, ObjId(20))
                ))
                .is_err()
            );
            assert_eq!(
                (e.action_state().revision(), count(&e, &f)),
                before,
                "actor ceiling is enforced before the global ceiling"
            );
        }
        let seller = ObjId(20 + (n / index::MAX_SELLER_LISTINGS) as u32);
        change(&mut e, &f, |t| {
            index::publish_listing(t, &f, listing_for(n as u128 + 1, seller))
        })
        .expect("within real cap");
    }
    assert_eq!(count(&e, &f).listings as usize, index::MAX_LISTINGS);
    let before = (
        e.action_state().revision(),
        e.action_state().len(),
        count(&e, &f),
    );
    for seller in [ObjId(20), ObjId(99)] {
        assert!(
            change(&mut e, &f, |t| index::publish_listing(
                t,
                &f,
                listing_for(9999, seller)
            ))
            .is_err()
        );
        assert_eq!(
            (
                e.action_state().revision(),
                e.action_state().len(),
                count(&e, &f)
            ),
            before
        );
    }
    change(&mut e, &f, |t| {
        index::retire_listing(t, &f, id(1), ObjId(20))
    })
    .expect("free one slot");
    change(&mut e, &f, |t| {
        index::publish_listing(t, &f, listing_for(9999, ObjId(20)))
    })
    .expect("slot reusable");
    boot(&mut e).expect("full index recovery agrees");
}

#[test]
fn buyer_quote_cap_and_global_cap_retire_without_touching_another_owner() {
    let (mut e, f) = rig();
    for n in 0..index::MAX_QUOTES {
        if n == index::MAX_BUYER_QUOTES {
            let before = (e.action_state().revision(), count(&e, &f));
            assert!(
                change(&mut e, &f, |t| index::publish_quote(
                    t,
                    &f,
                    quote_for(9999, ObjId(100))
                ))
                .is_err()
            );
            assert_eq!(
                (e.action_state().revision(), count(&e, &f)),
                before,
                "buyer ceiling is independent of global capacity"
            );
        }
        let buyer = ObjId(100 + (n / index::MAX_BUYER_QUOTES) as u32);
        change(&mut e, &f, |t| {
            index::publish_quote(t, &f, quote_for(n as u128 + 1, buyer))
        })
        .expect("within real cap");
    }
    let before = (e.action_state().revision(), count(&e, &f));
    for buyer in [ObjId(100), ObjId(999)] {
        assert!(
            change(&mut e, &f, |t| index::publish_quote(
                t,
                &f,
                quote_for(9999, buyer)
            ))
            .is_err()
        );
        assert_eq!((e.action_state().revision(), count(&e, &f)), before);
    }
    assert!(
        change(&mut e, &f, |t| index::retire_quotes(
            t,
            &f,
            &[id(1)],
            Some(ObjId(101)),
            160
        ))
        .is_err()
    );
    assert_eq!((e.action_state().revision(), count(&e, &f)), before);
    change(&mut e, &f, |t| {
        index::retire_quotes(t, &f, &[id(1)], Some(ObjId(100)), 100)
    })
    .expect("owner may retire before expiry");
    change(&mut e, &f, |t| {
        index::publish_quote(t, &f, quote_for(9999, ObjId(100)))
    })
    .expect("quote slot reusable");
    boot(&mut e).expect("all real quote/expiry/count joins");
}

#[test]
fn expiry_is_bounded_atomic_and_checks_actual_deadline_and_index() {
    let (mut e, f) = rig();
    for n in 0..17 {
        change(&mut e, &f, |t| {
            index::publish_quote(t, &f, quote_for(n + 1, ObjId(20 + (n / 8) as u32)))
        })
        .expect("seed quote");
    }
    let first: Vec<_> = (1..=16).map(id).collect();
    let before = (e.action_state().revision(), count(&e, &f));
    assert!(
        change(&mut e, &f, |t| index::retire_quotes(
            t, &f, &first, None, 159
        ))
        .is_err()
    );
    assert!(
        change(&mut e, &f, |t| index::retire_quotes(
            t,
            &f,
            &(1..=17).map(id).collect::<Vec<_>>(),
            None,
            160
        ))
        .is_err()
    );
    assert_eq!((e.action_state().revision(), count(&e, &f)), before);
    let writes = change(&mut e, &f, |t| {
        index::retire_quotes(t, &f, &first, None, 160)
    })
    .expect("exact boundary expiry");
    assert_eq!(
        writes.len(),
        35,
        "32 row deletions, two actor deletions and one count postimage"
    );
    assert_eq!(count(&e, &f).quotes, 1);
    assert!(
        e.action_state()
            .get(&f.quotes, &id(17))
            .expect("read")
            .is_some()
    );
    boot(&mut e).expect("retired indexes exact");
}

#[test]
fn registered_retirement_requires_actual_owner_and_expiry_god_cannot_retire_early() {
    let (mut e, f) = rig();
    change(&mut e, &f, |t| {
        index::publish_listing(t, &f, listing_for(40, mush_core::GOD))
    })
    .expect("own listing");
    change(&mut e, &f, |t| {
        index::publish_listing(t, &f, listing_for(41, ObjId(7)))
    })
    .expect("foreign listing");
    let action = config().retire_listing_action().expect("action");
    assert!(
        e.prepare_registered_composed_authoritative_action(
            &action,
            &invocation(json!({"publication":id(41)})),
            id(42)
        )
        .is_err()
    );
    let (prepared, _) = e
        .prepare_registered_composed_authoritative_action(
            &action,
            &invocation(json!({"publication":id(40)})),
            id(43),
        )
        .expect("actual owner retirement");
    let (_, install) = prepared.into_parts();
    e.install_composed_action(install)
        .expect("install all indexes");
    assert!(
        e.action_state()
            .get(&f.listings, &id(40))
            .expect("read")
            .is_none()
    );
    assert!(
        e.action_state()
            .get(&f.listings, &id(41))
            .expect("read")
            .is_some()
    );
    change(&mut e, &f, |t| {
        index::publish_quote(t, &f, quote_for(44, ObjId(8)))
    })
    .expect("quote");
    let expiry = config().action_ref(EXPIRE_ACTION).expect("expiry action");
    e.world.set_now(159);
    assert!(
        e.prepare_registered_composed_authoritative_action(
            &expiry,
            &invocation(json!({"quotes":[id(44)]})),
            id(45)
        )
        .is_err()
    );
    e.world.set_now(160);
    assert!(
        e.prepare_registered_composed_authoritative_action(
            &expiry,
            &invocation_as(json!({"quotes":[id(44)]}), ObjId(8)),
            id(46)
        )
        .is_err(),
        "a buyer cannot invoke the service expiry authority"
    );
    assert!(
        e.prepare_registered_composed_authoritative_action(
            &expiry,
            &invocation(json!({"quotes":[id(44),id(44)]})),
            id(46)
        )
        .is_err(),
        "duplicate batch IDs refuse before staging"
    );
    let (prepared, _) = e
        .prepare_registered_composed_authoritative_action(
            &expiry,
            &invocation(json!({"quotes":[id(44)]})),
            id(46),
        )
        .expect("actual expiry action");
    let (_, install) = prepared.into_parts();
    e.install_composed_action(install).expect("expiry install");
    assert_eq!(count(&e, &f).quotes, 0);
}

#[test]
fn registered_expiry_tick_projects_bounded_work_and_skips_pending_god() {
    use mush_core::action::{
        IdempotencyKey, IdempotencyScope, PerceivedRef, RequestUid, ScopedIdempotencyKey,
    };
    use mush_core::{OfferExecutionContext, OfferSubmission};

    fn fire(e: &mut Engine, now: i64) -> Outbox {
        e.world.set_now(now);
        let mut out = Outbox::new();
        e.fire_due(now * 1000, &mut out, &mut mush_core::engine::Effects::new());
        out
    }
    fn land(
        e: &mut Engine,
        f: &Families,
        submit: &mush_core::AuthoritativeSubmit,
        serial: u8,
        expected: &[CommitToken],
    ) {
        let principal = mush_core::PrincipalId::Object { id: submit.actor };
        let query = e
            .legal_action_query(principal.clone(), submit.actor, submit.subject.clone())
            .expect("actual actor projection");
        let offers = e
            .project_legal_actions(&query)
            .expect("registered providers");
        let projected = offers
            .projected_for_action(&submit.action)
            .expect("actual due expiry offer");
        assert_eq!(projected.subject(), &submit.subject);
        let resolved = e
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request: RequestUid::from_bytes([serial; 16]),
                    principal: principal.clone(),
                    actor: submit.actor,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal,
                            owner: submit.action.key().owner.clone(),
                            namespace: CanonicalId::new(submit.action.key().id.as_str())
                                .expect("registered action namespace"),
                        },
                        1,
                        IdempotencyKey::Caller(submit.key.clone()),
                    ),
                    viewer_scope: query.viewer_scope(),
                    now: None,
                },
                OfferSubmission {
                    offer: projected.offer(),
                    subject: submit.subject.clone(),
                    targets: submit.targets.clone(),
                    parameters: submit.parameters.clone(),
                },
            )
            .expect("ordinary fixed-parameter resolution");
        let (action, resolved, _) = resolved.into_authoritative_parts();
        assert_eq!(action, submit.action);
        assert_eq!(
            resolved.invocation().parameters(),
            &json!({"quotes":expected})
        );
        let before = count(e, f);
        let (prepared, _) = e
            .prepare_registered_composed_resolved_authoritative_action(
                &action,
                &resolved,
                id(u128::from(serial) + 1000),
            )
            .expect("real projected expiry preparation");
        assert_eq!(count(e, f), before, "preparation cannot retire live rows");
        let (writes, install) = prepared.into_parts();
        assert!(!writes.is_empty());
        e.install_composed_action(install)
            .expect("install entire expiry batch");
        assert_eq!(count(e, f).quotes, before.quotes - expected.len() as u32);
        for quote in expected {
            assert!(
                e.action_state()
                    .get(&f.quotes, quote)
                    .expect("read")
                    .is_none()
            );
        }
        boot(e).expect("all post-install indexes agree");
    }

    let (mut e, f) = rig();
    e.set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x41; 32]).expect("offer secret"),
            [],
        )
        .expect("offer key ring"),
    );
    for n in 1..=17 {
        change(&mut e, &f, |t| {
            index::publish_quote(t, &f, quote_for(n, ObjId(100 + ((n - 1) / 8) as u32)))
        })
        .expect("bounded actual due rows");
    }
    let revision = e.action_state().revision();
    fire(&mut e, 155);
    assert!(!e.has_authoritative_submit(), "deadline has not arrived");
    fire(&mut e, 160);
    assert!(e.has_authoritative_submit_for(mush_core::GOD));
    assert_eq!(count(&e, &f).quotes, 17);
    assert_eq!(e.action_state().revision(), revision);

    // The first request remains queued in the real Engine. A later registered
    // tick must skip it, rather than attempt a second submission/backpressure.
    let skipped = fire(&mut e, 165);
    assert!(
        skipped.is_empty(),
        "pending GOD must not receive a retry refusal"
    );
    let first = e
        .take_authoritative_submit()
        .expect("first real timer request");
    assert_eq!(first.actor, mush_core::GOD);
    assert_eq!(
        first.action,
        config().action_ref(EXPIRE_ACTION).expect("action")
    );
    assert_eq!(first.subject, PerceivedRef::SelfActor);
    assert!(first.targets.is_empty());
    assert_eq!(
        first.parameters,
        json!({}),
        "timer cannot invent its due batch"
    );
    assert_eq!(
        first.key.as_str(),
        format!("market.mind.expire.{}.{:x}", id(1).to_hex(), 160)
    );
    assert!(e.take_authoritative_submit().is_none());
    let expected = (1..=16).map(id).collect::<Vec<_>>();
    land(&mut e, &f, &first, 1, &expected);
    assert_eq!(count(&e, &f).quotes, 1);

    fire(&mut e, 170);
    let remaining = e
        .take_authoritative_submit()
        .expect("remaining due work retried");
    land(&mut e, &f, &remaining, 2, &[id(17)]);
    assert_eq!(count(&e, &f).quotes, 0);
    fire(&mut e, 175);
    assert!(
        !e.has_authoritative_submit(),
        "empty index produces no work"
    );
}

fn query(e: &mut Engine, actor: ObjId, args: Value) -> Value {
    let mut out = Outbox::new();
    e.step(
        mush_core::Inbound::Query {
            conn: 1,
            actor,
            id: 99,
            name: QUERY.into(),
            args,
        },
        &mut out,
        &mut mush_core::engine::Effects::new(),
    );
    out.events()
        .iter()
        .find_map(|(_, event)| match event {
            mush_proto::OutputEvent::Reply { id: 99, event } => match event.as_ref() {
                mush_proto::OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("actual connection-correlated query panel")
}
#[test]
fn actual_queries_page_own_rows_and_do_not_disclose_private_subject_or_foreign_quotes() {
    let (mut a, fa) = rig();
    let (mut b, fb) = rig();
    for (engine, f) in [(&mut a, &fa), (&mut b, &fb)] {
        for n in 1..=18 {
            change(engine, f, |t| {
                index::publish_listing(t, f, listing_for(n, mush_core::GOD))
            })
            .expect("own listing");
        }
        change(engine, f, |t| {
            index::publish_quote(t, f, quote_for(50, mush_core::GOD))
        })
        .expect("own quote");
    }
    change(&mut b, &fb, |t| {
        index::publish_quote(t, &fb, quote_for(70, ObjId(8)))
    })
    .expect("only private foreign difference");
    let first = query(&mut a, mush_core::GOD, json!({"kind":"own"}));
    assert_eq!(first, query(&mut b, mush_core::GOD, json!({"kind":"own"})));
    assert_eq!(first["listings"].as_array().expect("rows").len(), 16);
    let second = query(
        &mut a,
        mush_core::GOD,
        json!({"kind":"own","after":first["after"]}),
    );
    assert_eq!(second["listings"].as_array().expect("rows").len(), 2);
    assert_eq!(second["quotes"].as_array().expect("rows").len(), 1);
    let public = query(&mut a, mush_core::GOD, json!({"kind":"listings"}));
    assert_eq!(
        public,
        query(&mut b, mush_core::GOD, json!({"kind":"listings"}))
    );
    for data in [&first, &second, &public] {
        let text = data.to_string();
        for hidden in [
            "seller",
            "buyer",
            "payer",
            "account",
            "rating",
            "receipt_digest",
            "compatibility",
            "source",
            "definition",
            "charter",
            "owner_licence",
        ] {
            assert!(!text.contains(hidden), "unexpected private field {hidden}");
        }
        assert!(!text.contains(&hash().to_string()));
    }
}

#[test]
fn restart_audit_refuses_missing_counter_actor_or_expiry_and_orphan_index() {
    let (mut e, f) = rig();
    let mut disk = BTreeMap::new();
    for ops in [
        change(&mut e, &f, |t| {
            index::publish_listing(t, &f, listing_for(1, ObjId(7)))
        })
        .expect("listing"),
        change(&mut e, &f, |t| {
            index::publish_quote(t, &f, quote_for(2, ObjId(8)))
        })
        .expect("quote"),
    ] {
        for op in ops {
            match op.op {
                ModuleWrite::JsonSet { key, value } => {
                    disk.insert(key.to_string(), value);
                }
                ModuleWrite::Del { key } => {
                    disk.remove(key.as_str());
                }
                _ => panic!("typed fixture emitted non-document mutation"),
            }
        }
    }
    let restore = |rows: &BTreeMap<String, Value>| {
        let (mut engine, _) = rig();
        engine
            .hydrate_action_state(rows.iter().map(|(key, value)| {
                mush_core::ActionStateBootRecord::new(crate::NS, key.clone(), value.clone())
            }))
            .expect("valid individual typed rows");
        boot(&mut engine)
    };
    restore(&disk).expect("exact real writer postimages restart");
    for family in [
        index::Counts::FAMILY,
        index::ActorOffers::FAMILY,
        index::Expiry::FAMILY,
        MindListing::FAMILY,
        MindQuote::FAMILY,
    ] {
        let mut damaged = disk.clone();
        damaged.retain(|key, _| !key.contains(family));
        assert!(
            restore(&damaged).is_err(),
            "missing {family} must not look healthy"
        );
    }
}

#[test]
fn expiry_index_orders_the_entire_signed_utc_range() {
    let keys: Vec<_> = [i64::MIN, -1, 0, 1, i64::MAX]
        .into_iter()
        .map(|at| index::ExpiryKey { at, quote: id(1) }.encoded())
        .collect();
    assert!(keys.windows(2).all(|p| p.first() < p.get(1)));
}
