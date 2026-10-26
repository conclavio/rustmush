//! ---- Doc 18 §9 risk 19, cut IC14 row 1: the applied-marker ring ----
//!
//! `inventory.applied` had no retention past `MAX_APPLIED_MARKERS` = 4,096: past it
//! the mirror is incomplete and every inventory fact answers nothing (E7c2 fold R3's
//! fail-closed rule, which stands). Cut IC14 **bounds** the family per source
//! endpoint with a [`MarkerRing`] that evicts only past
//! [`MAX_RETAINED_MARKERS_PER_ENDPOINT`] entries **and** past
//! [`MARKER_RETENTION_SECS`] of real age, and refuses the next move once
//! [`MAX_MARKER_RING`] young entries fill it — E7c2 owner decision D1 (the marker is
//! the reboot-surviving replay guard) holds inside the horizon.
//!
//! The rig is `ic10.rs`'s: `boot()` composes the module, `apply()` opens the
//! owner-stamped transaction a provider gets and installs it exactly as an exact
//! receipt does. The real clock is the `wall` argument the prepare would read off the
//! offer's fixed parameters; nothing sleeps. One test lands a real `@inv/move` through
//! the provider to prove the offer carries the stamp.

use mush_core::{
    ClockRef, ClockStamp, CommitToken, Engine, GOD, IdempotencyKey, IdempotencyScope,
    OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{FactProvider, VaultId};
use mush_inventory::{
    EndpointRef, InventoryError, InventoryFactProvider, InventoryFamilies, InventoryHandles,
    InventoryModule, InventoryStore, MARKER_RETENTION_SECS, MAX_APPLIED_MARKERS,
    MAX_MARKER_ID_BYTES, MAX_MARKER_RING, MAX_RETAINED_MARKERS_PER_ENDPOINT,
    MAX_RING_EVICTIONS_PER_PUSH, MarkerRing, RingEntry, derived_marker_id, is_marker_id,
};
use mush_testkit::TestWorld;
use serde_json::json;
use smol_str::SmolStr;

/// The real second the first move's offer was drafted.
const T0: i64 = 1_700_000_000;

fn boot() -> (TestWorld, InventoryFamilies) {
    let world = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    let families = world
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    (world, families)
}

fn commodity() -> mush_core::CanonicalId {
    mush_core::CanonicalId::new("metals").expect("test commodity id")
}

fn endpoint(id: u64) -> EndpointRef {
    EndpointRef(VaultId(id))
}

fn now() -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 1_000,
    }
}

fn apply<T>(
    engine: &mut Engine,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, InventoryError>,
) -> Result<T, InventoryError> {
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        let result = body(&mut txn);
        match result {
            Ok(value) => (Ok(value), txn.prepare().ok()),
            Err(error) => (Err(error), None),
        }
    };
    if let Some(prepared) = prepared {
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
    }
    result
}

fn read<T>(
    engine: &mut Engine,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> T,
) -> T {
    let mut txn = engine
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    body(&mut txn)
}

/// Credit `amount` metals into each endpoint, a batch per bundle.
fn seed(engine: &mut Engine, families: &InventoryFamilies, at: &[EndpointRef], amount: u64) {
    for chunk in at.chunks(40) {
        apply(engine, families, |txn| {
            for &endpoint in chunk {
                families.credit(txn, endpoint, &commodity(), amount)?;
            }
            Ok(())
        })
        .expect("seed credits");
    }
}

/// The marker id of the `n`-th move out of `from`.
fn marker(from: EndpointRef, n: u64) -> SmolStr {
    derived_marker_id(
        1_000,
        &json!({"kind": "ic14.move", "from": from.vault().0, "n": n}),
    )
    .expect("derives")
}

/// One marked move of 1 metals out of `from`, its offer drafted at `wall`.
fn go(
    engine: &mut Engine,
    families: &InventoryFamilies,
    from: EndpointRef,
    to: EndpointRef,
    id: &SmolStr,
    wall: i64,
) -> Result<(), InventoryError> {
    apply(engine, families, |txn| {
        families
            .transfer_marked(txn, from, to, &commodity(), 1, id, GOD, &now(), wall)
            .map(|_| ())
    })
}

fn ring(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef) -> MarkerRing {
    read(engine, families, |txn| {
        txn.get(&families.marker_ring, &at)
            .expect("ring reads")
            .cloned()
            .unwrap_or_default()
    })
}

fn applied(engine: &mut Engine, families: &InventoryFamilies, id: &SmolStr) -> bool {
    read(engine, families, |txn| {
        txn.get(&families.applied, id).expect("reads").is_some()
    })
}

fn balance(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef) -> u64 {
    read(engine, families, |txn| {
        families
            .pool(txn, at, &commodity())
            .expect("pool reads")
            .map_or(0, |pool| pool.quantity)
    })
}

/// 40 moves out of one hold, a minute apart, inside one real hour.
fn forty_young_moves(
    engine: &mut Engine,
    families: &InventoryFamilies,
    hold: EndpointRef,
    store: EndpointRef,
) -> Vec<SmolStr> {
    (0..40)
        .map(|n| {
            let id = marker(hold, n);
            go(engine, families, hold, store, &id, T0 + 60 * n as i64).expect("a young move");
            id
        })
        .collect()
}

#[test]
fn ic14_a_young_ring_grows_past_32_and_evicts_nothing() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hold, store) = (endpoint(1), endpoint(2));
    seed(engine, &families, &[hold], 1_000);
    let ids = forty_young_moves(engine, &families, hold, store);
    let ring = ring(engine, &families, hold);
    assert_eq!(ring.len(), 40, "nothing younger than a real day is evicted");
    let listed: Vec<&SmolStr> = ring.entries().iter().map(|e| &e.key).collect();
    assert_eq!(listed, ids.iter().collect::<Vec<_>>(), "oldest first");
    for id in &ids {
        assert!(applied(engine, &families, id), "{id} is still applied");
    }
    // The destination keeps no ring: a transfer's marker belongs to its source.
    assert!(self::ring(engine, &families, store).is_empty());
}

#[test]
fn ic14_an_aged_ring_drains_to_32() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hold, store) = (endpoint(1), endpoint(2));
    seed(engine, &families, &[hold], 1_000);
    let ids = forty_young_moves(engine, &families, hold, store);
    let later = T0 + MARKER_RETENTION_SECS + 3_600;
    // The first aged move: 41 entries, at most eight evicted in its own bundle.
    let first = marker(hold, 100);
    go(engine, &families, hold, store, &first, later).expect("an aged move");
    let after_one = ring(engine, &families, hold);
    assert_eq!(after_one.len(), 41 - MAX_RING_EVICTIONS_PER_PUSH);
    for id in &ids[..MAX_RING_EVICTIONS_PER_PUSH] {
        assert!(
            !applied(engine, &families, id),
            "{id} was evicted from inventory.applied with its ring entry"
        );
    }
    assert!(applied(
        engine,
        &families,
        &ids[MAX_RING_EVICTIONS_PER_PUSH]
    ));
    // The second: 34 entries, two more evicted, and the floor holds at 32.
    let second = marker(hold, 101);
    go(engine, &families, hold, store, &second, later + 60).expect("another aged move");
    let after_two = ring(engine, &families, hold);
    assert_eq!(after_two.len(), MAX_RETAINED_MARKERS_PER_ENDPOINT);
    for id in &ids[..10] {
        assert!(!applied(engine, &families, id));
    }
    for id in ids[10..].iter().chain([&first, &second]) {
        assert!(applied(engine, &families, id), "{id} is retained");
    }
    // At the floor nothing is evicted however old: a third move adds one and the
    // ring evicts exactly one, never dipping below 32.
    let third = marker(hold, 102);
    go(engine, &families, hold, store, &third, later + 120).expect("a third aged move");
    assert_eq!(
        ring(engine, &families, hold).len(),
        MAX_RETAINED_MARKERS_PER_ENDPOINT
    );
}

#[test]
fn ic14_a_transferred_marker_survives_40_later_moves_inside_the_horizon() {
    let (mut world, families) = boot();
    let (hold, house) = (endpoint(1), endpoint(2));
    let sale = marker(hold, 999);
    {
        let engine = world.engine_mut();
        seed(engine, &families, &[hold], 1_000);
        go(engine, &families, hold, house, &sale, T0).expect("the sale's leg lands");
        for n in 0..40 {
            let id = marker(hold, n);
            go(
                engine,
                &families,
                hold,
                endpoint(3),
                &id,
                T0 + 60 + 60 * n as i64,
            )
            .expect("a later move");
        }
        assert!(applied(engine, &families, &sale), "D1: still the guard");
    }
    world.modules_loaded(std::collections::HashMap::new());
    let state = InventoryFactProvider
        .resolve(world.world(), mush_inventory::FACT_TRANSFERRED, &sale, None)
        .expect("the sale's fact still answers");
    assert!(state.satisfied);
    // The HIT: a replay of the same marked transfer is refused, and moves nothing.
    let engine = world.engine_mut();
    let before = balance(engine, &families, hold);
    assert_eq!(
        go(engine, &families, hold, house, &sale, T0),
        Err(InventoryError::AlreadyExists)
    );
    assert_eq!(balance(engine, &families, hold), before);
}

#[test]
fn ic14_a_full_young_ring_refuses_the_next_move() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hold, store) = (endpoint(1), endpoint(2));
    seed(engine, &families, &[hold], 1_000);
    let ids: Vec<SmolStr> = (0..MAX_MARKER_RING as u64)
        .map(|n| {
            let id = marker(hold, n);
            go(engine, &families, hold, store, &id, T0 + n as i64).expect("a young move");
            id
        })
        .collect();
    assert_eq!(ring(engine, &families, hold).len(), MAX_MARKER_RING);
    let next = marker(hold, 10_000);
    let before = balance(engine, &families, hold);
    assert_eq!(
        go(engine, &families, hold, store, &next, T0 + 3_600),
        Err(InventoryError::MarkerRingFull),
        "fail closed: never evict a young marker"
    );
    assert_eq!(
        InventoryError::MarkerRingFull.to_string(),
        "this endpoint has recorded too many moves in the last day to record another; try \
         again later"
    );
    assert_eq!(balance(engine, &families, hold), before, "nothing moved");
    assert!(!applied(engine, &families, &next), "nothing was written");
    assert_eq!(ring(engine, &families, hold).len(), MAX_MARKER_RING);
    for id in &ids {
        assert!(applied(engine, &families, id), "{id} is intact");
    }
}

#[test]
fn ic14_the_mirror_stays_complete_past_4096_lifetime_markers() {
    let (mut world, families) = boot();
    let holds: Vec<EndpointRef> = (0..120).map(|i| endpoint(100 + i)).collect();
    let mut newest = None;
    let mut lifetime = 0_usize;
    {
        let engine = world.engine_mut();
        seed(engine, &families, &holds, 100);
        // 40 young moves, then two aged ones, out of every hold: 42 markers each.
        for n in 0..42_u64 {
            let wall = if n < 40 {
                T0 + 60 * n as i64
            } else {
                T0 + MARKER_RETENTION_SECS + 60 * n as i64
            };
            for chunk in holds.chunks(40) {
                apply(engine, &families, |txn| {
                    for &hold in chunk {
                        let id = marker(hold, n);
                        let to = endpoint(10_000 + hold.vault().0);
                        families.transfer_marked(
                            txn,
                            hold,
                            to,
                            &commodity(),
                            1,
                            &id,
                            GOD,
                            &now(),
                            wall,
                        )?;
                    }
                    Ok(())
                })
                .expect("a batch of moves");
                lifetime += chunk.len();
            }
            newest = Some(marker(holds[119], n));
        }
        for &hold in &holds {
            assert_eq!(
                ring(engine, &families, hold).len(),
                MAX_RETAINED_MARKERS_PER_ENDPOINT
            );
        }
    }
    assert!(
        lifetime > MAX_APPLIED_MARKERS,
        "{lifetime} lifetime markers"
    );
    world.modules_loaded(std::collections::HashMap::new());
    let store = world
        .world()
        .module_state::<InventoryStore>()
        .expect("store");
    assert!(store.is_complete(), "the mirror sees the whole family");
    assert_eq!(store.markers(), 120 * MAX_RETAINED_MARKERS_PER_ENDPOINT);
    let newest = newest.expect("a newest marker");
    let state = InventoryFactProvider
        .resolve(
            world.world(),
            mush_inventory::FACT_TRANSFERRED,
            &newest,
            None,
        )
        .expect("the newest fact resolves");
    assert!(state.satisfied);
}

#[test]
fn ic14_a_ring_over_its_cap_does_not_decode() {
    let entry = |n: usize| json!({"key": format!("{n:080x}"), "wall": T0});
    let at_cap: Vec<_> = (0..MAX_MARKER_RING).map(entry).collect();
    assert!(serde_json::from_value::<MarkerRing>(json!(at_cap)).is_ok());
    let over: Vec<_> = (0..=MAX_MARKER_RING).map(entry).collect();
    assert!(serde_json::from_value::<MarkerRing>(json!(over)).is_err());
    // A duplicate key, a bad key and an unknown field refuse too.
    assert!(serde_json::from_value::<MarkerRing>(json!([entry(1), entry(1)])).is_err());
    assert!(serde_json::from_value::<MarkerRing>(json!([{"key": "XYZ", "wall": T0}])).is_err());
    assert!(
        serde_json::from_value::<MarkerRing>(json!([{"key": "ab", "wall": T0, "x": 1}])).is_err()
    );
    assert!(
        MarkerRing::new(vec![RingEntry {
            key: SmolStr::new(""),
            wall: T0,
            keep_s: None,
        }])
        .is_err()
    );
}

/// **`MarkerRing::admits` is `push`'s rule, read only** (doc 20 cut SM.11, the lead's pre-commit
/// round, item 1): over a ring at its cap of young entries, a ring with room, a ring whose oldest
/// entries aged past the horizon and a ring that ages by fewer than one push evicts, `admits(n,
/// now)` answers exactly whether `n` pushes stamped `now` all succeed. **Doc 20 cut SM.12c**
/// (decision (c)): re-blessed over rings mixing default and kept horizons — every entry kept an
/// hour, kept entries an hour old behind one young default entry (head of line), and kept entries
/// aged past their hour but not past the day ahead of default ones.
#[test]
fn sm11_admits_answers_exactly_what_push_would() {
    const HOUR: i64 = 3_600;
    let entry = |n: usize, (wall, keep_s): (i64, Option<i64>)| RingEntry {
        key: SmolStr::new(format!("{n:080x}")),
        wall,
        keep_s,
    };
    let ring_of = |entries: &[(i64, Option<i64>)]| {
        MarkerRing::new(
            entries
                .iter()
                .enumerate()
                .map(|(n, pair)| entry(n, *pair))
                .collect(),
        )
        .expect("a ring")
    };
    let pushes = |ring: &MarkerRing, n: usize, now: i64| {
        let mut ring = ring.clone();
        for k in 0..n {
            match ring.push(entry(100_000 + k, (now, None)), now) {
                Ok((next, _)) => ring = next,
                Err(_) => return false,
            }
        }
        true
    };
    let default = |wall: i64| (wall, None);
    let kept = |wall: i64| (wall, Some(HOUR));
    let young = vec![default(T0); MAX_MARKER_RING];
    let room = vec![default(T0); MAX_MARKER_RING - 2];
    let aged = vec![default(T0 - MARKER_RETENTION_SECS); MAX_MARKER_RING];
    // Only a few of the full ring's entries aged: fewer than one push may evict.
    let mut few = vec![default(T0); MAX_MARKER_RING];
    few[0] = default(T0 - MARKER_RETENTION_SECS);
    let kept_young = vec![kept(T0); MAX_MARKER_RING];
    let kept_aged = vec![kept(T0 - HOUR); MAX_MARKER_RING];
    // One young default entry ahead of hour-old kept ones holds them all.
    let mut held = vec![kept(T0 - HOUR); MAX_MARKER_RING];
    held[0] = default(T0);
    // Kept entries past their hour ahead of young default ones: only the kept may go.
    let mut mixed = vec![default(T0); MAX_MARKER_RING];
    for slot in mixed.iter_mut().take(5) {
        *slot = kept(T0 - HOUR);
    }
    let rings = [
        &young,
        &room,
        &aged,
        &few,
        &kept_young,
        &kept_aged,
        &held,
        &mixed,
    ];
    for entries in rings {
        let ring = ring_of(entries);
        for n in 0..=MAX_RING_EVICTIONS_PER_PUSH + 2 {
            for now in [T0, T0 + 1, T0 + HOUR, T0 + MARKER_RETENTION_SECS] {
                assert_eq!(
                    ring.admits(n, now),
                    pushes(&ring, n, now),
                    "{} entries ({:?} first), {n} pushes at {now}",
                    entries.len(),
                    entries.first()
                );
            }
        }
    }
    assert!(!ring_of(&young).admits(1, T0), "a full young ring refuses");
    assert!(ring_of(&young).admits(0, T0), "nothing pushed fits");
    assert!(ring_of(&room).admits(2, T0) && !ring_of(&room).admits(3, T0));
    assert!(
        ring_of(&young).admits(1, T0 + MARKER_RETENTION_SECS),
        "the ring frees as its markers age"
    );
    assert!(
        ring_of(&kept_aged).admits(8, T0),
        "hour-old kept entries free the ring"
    );
    assert!(
        !ring_of(&held).admits(1, T0),
        "a young default entry ahead holds the ring"
    );
    assert!(
        ring_of(&mixed).admits(5, T0) && !ring_of(&mixed).admits(6, T0),
        "only the aged kept entries make room"
    );
}

#[test]
fn ic14_the_marker_id_rule_is_inventorys() {
    // The one rule (cut IC14 row 17): the market calls this, not a copy.
    assert!(is_marker_id("0123456789abcdef"));
    assert!(is_marker_id(&"a".repeat(MAX_MARKER_ID_BYTES)));
    assert!(!is_marker_id(&"a".repeat(MAX_MARKER_ID_BYTES + 1)));
    assert!(!is_marker_id(""));
    assert!(!is_marker_id("ABCDEF"));
    assert!(!is_marker_id("xyz"));
    let derived = derived_marker_id(1_000, &json!({"k": 1})).expect("derives");
    assert!(is_marker_id(&derived));
}

// ---- the provider's own path ---------------------------------------------------

fn econ_rig() -> (TestWorld, InventoryFamilies) {
    let world = TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(InventoryModule::new()),
    ]);
    let families = world
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    (world, families)
}

fn number_after(
    events: &[(mush_core::Audience, mush_proto::OutputEvent)],
    needle: &str,
) -> Option<u64> {
    for line in events.iter().map(|(_, e)| mush_testkit::text_of(e)) {
        if let Some(at) = line.find(needle) {
            let digits: String = line[at + needle.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !digits.is_empty() {
                return digits.parse().ok();
            }
        }
    }
    None
}

fn make_vault(world: &mut TestWorld, name: &str) -> EndpointRef {
    let created = world.run(GOD, &format!("@create {name}"));
    let object = number_after(&created, "(#").expect("the created object's id");
    let out = world.run(GOD, &format!("@vault/create {name}=container:#{object}"));
    EndpointRef(VaultId(number_after(&out, "vault #").expect("vault id")))
}

/// Drive one queued submission the way the world task does (the market's
/// `land_submit` pattern): resolve it against the live offer, prepare, install.
fn land(world: &mut TestWorld, token: u8) -> Result<(), String> {
    let submit = *world
        .engine_mut()
        .take_authoritative_submit()
        .ok_or("nothing queued")?;
    let (action, resolved) = {
        let engine = world.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(&submit.action)
            .ok_or_else(|| "that action is not currently offered".to_owned())?;
        let submission = OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: submit.parameters.clone(),
        };
        let execution = OfferExecutionContext {
            request: RequestUid::from_bytes([token; 16]),
            principal: mush_core::PrincipalId::Object { id: submit.actor },
            actor: submit.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: submit.actor },
                    owner: submit.action.key().owner.clone(),
                    namespace: mush_core::CanonicalId::new(submit.action.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|error| format!("resolve: {error:?}"))?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    let (prepared, _) = world
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .map_err(|error| format!("prepare: {error:?}"))?;
    world
        .engine_mut()
        .install_composed_action(prepared.into_parts().1)
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

/// **The offer carries the real stamp**: a staff `@inv/move` landed through the
/// provider records its marker in the source endpoint's ring, stamped with the real
/// second the offer was drafted (the `"wall"` fixed parameter beside `"now"`).
#[test]
fn ic14_a_provider_move_records_its_marker_in_the_source_ring() {
    let (mut world, families) = econ_rig();
    world.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x41; 32]).expect("an offer secret"),
            [],
        )
        .expect("an offer key ring"),
    );
    world.world_mut().set_game_clock(mush_core::GameClock {
        ic_ratio: 1,
        epoch_offset: 0,
    });
    world.set_now(T0);
    let hold = make_vault(&mut world, "hold");
    let store = make_vault(&mut world, "store");
    seed(world.engine_mut(), &families, &[hold], 10);
    let _ = world.run(
        GOD,
        &format!("@inv/move {}/metals:3={}", hold.vault().0, store.vault().0),
    );
    land(&mut world, 7).expect("the move lands");
    let engine = world.engine_mut();
    assert_eq!(balance(engine, &families, store), 3);
    let ring = ring(engine, &families, hold);
    assert_eq!(ring.len(), 1, "one marker, in its source's ring");
    let entry = &ring.entries()[0];
    assert_eq!(entry.wall, T0, "stamped with the offer's real second");
    assert!(applied(engine, &families, &entry.key));
}

// ---- doc 20 cut SM.12c, decision (c): a per-entry retention horizon -------------

/// One real hour: the horizon TacSim's unload finish keeps its hopper markers.
const HOUR: i64 = 3_600;

/// A ring entry `n` stamped `wall`, kept `keep_s`.
fn ring_entry(n: usize, wall: i64, keep_s: Option<i64>) -> RingEntry {
    RingEntry {
        key: SmolStr::new(format!("{n:080x}")),
        wall,
        keep_s,
    }
}

/// One marked move of 1 metals out of `from` through `transfer_many_marked_keeping`.
fn go_keeping(
    engine: &mut Engine,
    families: &InventoryFamilies,
    (from, to): (EndpointRef, EndpointRef),
    id: &SmolStr,
    wall: i64,
    keep_s: i64,
) -> Result<(), InventoryError> {
    apply(engine, families, |txn| {
        families
            .transfer_many_marked_keeping(
                txn,
                from,
                to,
                &[(commodity(), 1, id.clone())],
                GOD,
                &now(),
                wall,
                keep_s,
            )
            .map(|_| ())
    })
}

/// **`admits` honours a kept entry's horizon**: a full ring of entries kept an hour refuses
/// inside the hour and admits a push's worth of evictions at it; the same walls at the default
/// horizon still refuse at the hour.
#[test]
fn sm12c_admits_honours_a_kept_entrys_horizon() {
    let kept = MarkerRing::new(
        (0..MAX_MARKER_RING)
            .map(|n| ring_entry(n, T0, Some(HOUR)))
            .collect(),
    )
    .expect("a ring");
    let default = MarkerRing::new(
        (0..MAX_MARKER_RING)
            .map(|n| ring_entry(n, T0, None))
            .collect(),
    )
    .expect("a ring");
    assert!(
        !kept.admits(1, T0 + HOUR - 1),
        "a kept entry inside its hour holds"
    );
    assert!(kept.admits(1, T0 + HOUR), "a kept entry an hour old may go");
    assert!(
        kept.admits(MAX_RING_EVICTIONS_PER_PUSH, T0 + HOUR),
        "a push evicts up to its eight"
    );
    assert!(
        !default.admits(1, T0 + HOUR),
        "the default horizon is the day"
    );
    assert!(default.admits(1, T0 + MARKER_RETENTION_SECS));
}

/// **A kept entry ages out at its own horizon**, through the families: forty moves kept an hour,
/// then one more past the hour evicts up to its eight from the front and retires their applied
/// markers in the same bundle; the forty at the default horizon evict nothing at the same second.
#[test]
fn sm12c_a_kept_entry_ages_out_at_its_own_horizon() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hold, store, other) = (endpoint(1), endpoint(2), endpoint(3));
    seed(engine, &families, &[hold, other], 1_000);
    let ids: Vec<SmolStr> = (0..40)
        .map(|n| {
            let id = marker(hold, n);
            go_keeping(
                engine,
                &families,
                (hold, store),
                &id,
                T0 + 60 * n as i64,
                HOUR,
            )
            .expect("a kept move");
            id
        })
        .collect();
    let kept = ring(engine, &families, hold);
    assert_eq!(kept.len(), 40, "nothing younger than its hour is evicted");
    assert!(
        kept.entries().iter().all(|e| e.keep_s == Some(HOUR)),
        "each entry carries its horizon"
    );
    let later = T0 + 60 * 39 + HOUR;
    let next = marker(hold, 1_000);
    go(engine, &families, hold, store, &next, later).expect("a move past the hour");
    let ring_now = ring(engine, &families, hold);
    assert_eq!(ring_now.len(), 41 - MAX_RING_EVICTIONS_PER_PUSH);
    for id in &ids[..MAX_RING_EVICTIONS_PER_PUSH] {
        assert!(!applied(engine, &families, id), "{id} aged out at its hour");
    }
    for id in &ids[MAX_RING_EVICTIONS_PER_PUSH..] {
        assert!(applied(engine, &families, id), "{id} is still held");
    }
    assert_eq!(ring_now.entries().last().map(|e| e.keep_s), Some(None));
    // The same walls at the default horizon: nothing goes.
    for n in 0..40 {
        go(
            engine,
            &families,
            other,
            store,
            &marker(other, n),
            T0 + 60 * n as i64,
        )
        .expect("a default move");
    }
    go(
        engine,
        &families,
        other,
        store,
        &marker(other, 1_000),
        later,
    )
    .expect("one more");
    assert_eq!(
        ring(engine, &families, other).len(),
        41,
        "the default keeps the day"
    );
}

/// **A ring without horizons encodes as before**: an entry with no `keep_s` serializes to exactly
/// the pre-cut `{key, wall}`, a pre-cut ring decodes with every horizon absent and re-encodes
/// byte-identically, and a kept entry carries `keep_s`.
#[test]
fn sm12c_a_ring_without_horizons_encodes_as_before() {
    let key = format!("{:080x}", 7);
    let before = format!(r#"[{{"key":"{key}","wall":{T0}}}]"#);
    let ring: MarkerRing = serde_json::from_str(&before).expect("a pre-cut ring decodes");
    assert_eq!(ring.entries()[0].keep_s, None);
    assert_eq!(serde_json::to_string(&ring).expect("encodes"), before);
    assert_eq!(
        serde_json::to_value(ring_entry(7, T0, None)).expect("encodes"),
        json!({"key": key, "wall": T0})
    );
    assert_eq!(
        serde_json::to_value(ring_entry(7, T0, Some(HOUR))).expect("encodes"),
        json!({"key": key, "wall": T0, "keep_s": HOUR})
    );
    let kept: MarkerRing =
        serde_json::from_value(json!([{"key": key, "wall": T0, "keep_s": HOUR}])).expect("decodes");
    assert_eq!(kept.entries()[0].keep_s, Some(HOUR));
    for keep_s in [0, -1, MARKER_RETENTION_SECS + 1] {
        assert!(
            serde_json::from_value::<MarkerRing>(
                json!([{"key": key, "wall": T0, "keep_s": keep_s}])
            )
            .is_err(),
            "a stored horizon of {keep_s} does not decode"
        );
    }
}

/// **A horizon past the default is refused**: `keep_s` outside `1..=MARKER_RETENTION_SECS` is
/// `MarkerHorizon` with nothing staged (no balance moved, no marker, no ring); the day itself is
/// admitted.
#[test]
fn sm12c_a_horizon_past_the_default_is_refused() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hold, store) = (endpoint(1), endpoint(2));
    seed(engine, &families, &[hold], 10);
    for (n, keep_s) in [MARKER_RETENTION_SECS + 1, 0, -1, i64::MAX]
        .into_iter()
        .enumerate()
    {
        let id = marker(hold, n as u64);
        assert_eq!(
            go_keeping(engine, &families, (hold, store), &id, T0, keep_s),
            Err(InventoryError::MarkerHorizon),
            "keep_s {keep_s}"
        );
        assert!(!applied(engine, &families, &id));
    }
    assert_eq!(balance(engine, &families, hold), 10, "nothing moved");
    assert!(
        ring(engine, &families, hold).is_empty(),
        "nothing was ringed"
    );
    let id = marker(hold, 99);
    go_keeping(
        engine,
        &families,
        (hold, store),
        &id,
        T0,
        MARKER_RETENTION_SECS,
    )
    .expect("the day itself is a horizon");
    assert_eq!(
        ring(engine, &families, hold).entries()[0].keep_s,
        Some(MARKER_RETENTION_SECS)
    );
}

/// **A default entry ahead holds the ring** (head of line): one default-horizon entry ahead of
/// 127 kept ones, all stamped `T0`; past the kept hour a push still refuses `MarkerRingFull`
/// (eviction is FIFO from the front), and once the front entry is a day old the push evicts it and
/// up to seven kept ones behind it.
#[test]
fn sm12c_a_default_entry_ahead_holds_the_ring() {
    let mut entries = vec![ring_entry(0, T0, None)];
    entries.extend((1..MAX_MARKER_RING).map(|n| ring_entry(n, T0, Some(HOUR))));
    let ring = MarkerRing::new(entries).expect("a ring");
    let later = T0 + 2 * HOUR;
    assert!(!ring.admits(1, later));
    assert!(
        ring.push(ring_entry(500, later, Some(HOUR)), later)
            .is_err()
    );
    let day = T0 + MARKER_RETENTION_SECS;
    assert!(ring.admits(1, day));
    let (next, evicted) = ring
        .push(ring_entry(500, day, Some(HOUR)), day)
        .expect("the front aged");
    assert_eq!(evicted.len(), MAX_RING_EVICTIONS_PER_PUSH);
    assert_eq!(evicted[0], SmolStr::new(format!("{:080x}", 0)));
    assert_eq!(
        next.len(),
        MAX_MARKER_RING + 1 - MAX_RING_EVICTIONS_PER_PUSH
    );
}
