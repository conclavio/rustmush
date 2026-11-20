//! **Tier 1B security hardening regression tests — field offer sizing, bay_work fee contracts,
//! bay inspect gating, and bay strip hold-capacity checks** (CRIT-18, CRIT-19, HIGH-48, HIGH-52,
//! HIGH-53).

use mush_core::{GOD, ObjId, PerceivedRef};
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use mush_tactics::field_ops::ComponentCondition;
use mush_tactics::{ConditionId, UnitMutationOp};
use serde_json::json;
use smol_str::SmolStr;
use tacsim::port::HullRow;
use tacsim::port::bay_work::{BayHull, BayHullState, OrderStatus};

use crate::bay_gc8::{
    bay_room, component_id, damage, empty_magazine, magazine, mint, order_of, put, unit, worn,
};
use crate::driving_rig::{
    Rig, at_counter, drain, fire, hold_pool, hold_vault_of, land_all, land_quietly, pilot,
    prospector, reboot, rigs_of, run, seed_hold, station_room, stocked, store, to_ic,
};

const CERES: &str = "ceres_port";
const C14: &str = "room.c14_dockmaster";
const HOLD_FULL_T: u64 = 86;

fn bay_record(rig: &Rig, hull: ObjId) -> BayHull {
    match store(&rig.tw).bay_hull(hull) {
        Some(BayHullState::Present(row)) => row.clone(),
        other => panic!("bay record of #{}: {other:?}", hull.0),
    }
}

fn title_of(rig: &Rig, hull: ObjId) -> Option<PrincipalId> {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()))
}

fn spare_escort(rig: &Rig, like: ObjId) -> ObjId {
    let schema = unit(rig, like).schema().clone();
    rig.tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("units")
        .units
        .iter()
        .filter(|(id, u)| **id != like && u.schema() == &schema)
        .map(|(id, _)| *id)
        .find(|id| store(&rig.tw).tv_lease_on(*id).is_none())
        .expect("spare escort")
}

fn berthed_spare(rig: &mut Rig) -> (ObjId, HullRow, ObjId) {
    let (captain, row) = prospector(rig, "BayPilot", 97, "Iron Kite");
    mint(rig, captain);
    at_counter(rig, captain);
    let signed = run(&mut rig.tw, captain, "+tv/lease escort go");
    assert!(signed.contains("leases you OWP-ES-01"), "{signed}");
    let leased = rigs_of(&rig.tw, captain)[0];
    let spare = spare_escort(rig, leased);
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", spare.0, captain.0),
    );
    assert_eq!(
        title_of(rig, spare),
        Some(PrincipalId::Object(captain)),
        "{moved}"
    );
    put(rig, spare, bay_room(&row), true);
    worn(rig, spare);
    let workshop = station_room(&rig.tw, CERES, C14);
    put(rig, captain, workshop, false);
    let berthed = run(&mut rig.tw, captain, &format!("+bay/berth #{}", spare.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    (captain, row, spare)
}

fn crystallize(rig: &mut Rig, who: ObjId, vault: mush_economy::VaultId, good: &str, qty: u64) {
    let out = run(
        &mut rig.tw,
        who,
        &format!("@inv/crystallize #{}/{good}={qty}", vault.0),
    );
    let queued = drain(&mut rig.tw);
    assert!(!queued.is_empty(), "{out}");
    land_all(rig, &queued);
}

/// **CRIT-18**: Restoring an oversized persisted `FieldClosing` tombstone across restart (350
/// `Learned` rows with 120 `eyes` each + 12 carried minds totaling ~42 KiB + 1 oversized mind)
/// never locks out `Engine::project_legal_actions` for bystanders OR the closing owner; every
/// projected field offer stays strictly `<= 48 KiB`, chunks carried minds first, and degrades
/// gracefully when closing.
#[test]
fn crit18_composed_project_legal_actions_never_locks_out_on_oversized_field_close() {
    let mut rig = stocked();
    let owner = pilot(&mut rig, "FieldOwner", 98);
    let bystander = pilot(&mut rig, "FieldBystander", 99);

    let known_rows: Vec<serde_json::Value> = (1..=350_u32)
        .map(|p| {
            let items: Vec<serde_json::Value> = (0..20)
                .map(|i| {
                    serde_json::to_value(tacsim::surface::site_record::KnownItem(
                        tacsim::surface::site_record::KnownWhat::Casualty,
                        i,
                        i64::from(p),
                    ))
                    .expect("known item")
                })
                .collect();
            let eyes: Vec<(i64, i64)> = (0..120).map(|i| (i, i64::from(p))).collect();
            json!({
                "principal": p,
                "items": items,
                "eyes": eyes,
            })
        })
        .collect();
    let mem_map: std::collections::BTreeMap<String, mush_mind::memory::Value> = (0..6)
        .map(|key| {
            (
                format!("key{key:03}"),
                mush_mind::memory::Value::Ints(vec![i64::MIN; 32]),
            )
        })
        .collect();
    let mem = mush_mind::memory::encode(&mem_map);
    let minds: Vec<serde_json::Value> = (1..=12_u32)
        .map(|u| {
            serde_json::to_value(tacsim::field::record::CarriedRow {
                unit: ObjId(500 + u),
                epoch: tacsim::arena::mind::SortieId::Packed(u64::from(u)),
                mind: tacsim::arena::mind::CarriedMind {
                    seal_digest: [9; 32],
                    think_seq: u64::from(u),
                    memory: mem.clone(),
                    dropped: None,
                },
            })
            .expect("carried row")
        })
        .collect();

    let closing_key = SmolStr::new(format!(
        "{}ahuna_port",
        tacsim::field::record::CLOSING_PREFIX
    ));
    let tombstone = json!({
        "format": 1,
        "site": "ahuna_port",
        "close": {
            "actor": owner.0,
            "ordinal": 5,
            "units": [],
            "known": known_rows,
            "minds": minds,
        },
        "exits": [],
    });
    rig.store
        .entry(SmolStr::new_static("tacsim"))
        .or_default()
        .kv
        .push((closing_key.clone(), tombstone));

    // Reboot through the real boot path so `field::boot::read` restores & sanitizes the tombstone.
    let mut rig = reboot(rig, &[]);

    let field_action = rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .and_then(|h| h.field.clone())
        .expect("field action handle");

    let resolve_field_params = |rig: &Rig, actor: ObjId, op: &str, token_byte: u8| {
        let query = rig
            .tw
            .engine()
            .legal_action_query(
                mush_core::PrincipalId::Object { id: actor },
                actor,
                PerceivedRef::SelfActor,
            )
            .expect("query");
        let offers = rig
            .tw
            .engine()
            .project_legal_actions(&query)
            .expect("project_legal_actions must never error out globally");
        let projected = offers
            .projected_for_action(&field_action)
            .expect("field offer projected");
        let token = [token_byte; 16];
        let submission = mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: json!({ "site": "ahuna_port", "op": op }),
        };
        let execution = mush_core::OfferExecutionContext {
            request: mush_core::RequestUid::from_bytes(token),
            principal: mush_core::PrincipalId::Object { id: actor },
            actor,
            idempotency: mush_core::ScopedIdempotencyKey::new(
                mush_core::IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: actor },
                    owner: field_action.key().owner.clone(),
                    namespace: mush_core::CanonicalId::new(field_action.key().id.as_str())
                        .expect("namespace"),
                },
                1,
                mush_core::IdempotencyKey::Request(mush_core::RequestUid::from_bytes(token)),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = rig
            .tw
            .engine()
            .resolve_legal_action(&offers, execution, submission)
            .expect("resolve_legal_action succeeds under 64 KiB bound");
        let (_, resolved, _) = resolved.into_authoritative_parts();
        resolved.invocation().parameters().clone()
    };

    // 1. Bystander gets a tiny offer with empty closing/carried.
    let bystander_params = resolve_field_params(&rig, bystander, "open", 1);
    let bystander_bytes =
        mush_core::mechanics::canonical_json_bytes_with_limit(&bystander_params, 48 * 1024)
            .expect("bystander parameters <= 48 KiB");
    assert!(
        bystander_bytes.len() < 4096,
        "bystander offer should be small"
    );
    assert!(
        bystander_params["closing"]
            .as_object()
            .expect("closing")
            .is_empty()
    );
    assert!(
        bystander_params["carried"]
            .as_object()
            .expect("carried")
            .is_empty()
    );

    // 2. Owner gets a bounded `<= 48 KiB` carry-only chunk while carried minds exceed budget.
    let owner_carry = resolve_field_params(&rig, owner, "carry", 2);
    let carry_bytes =
        mush_core::mechanics::canonical_json_bytes_with_limit(&owner_carry, 48 * 1024)
            .expect("owner carry chunk <= 48 KiB");
    assert!(carry_bytes.len() <= 48 * 1024);
    assert!(
        !owner_carry["closing"]
            .as_object()
            .expect("closing")
            .contains_key("ahuna_port")
    );
    let chunk = owner_carry["carried"]["ahuna_port"]
        .as_array()
        .expect("carried chunk");
    assert!(!chunk.is_empty() && chunk.len() < 12);

    // 3. Replace tombstone with minds drained (still 350 huge `known` rows) and reboot:
    // owner gets a bounded `<= 48 KiB` degraded closing offer for `ahuna_port` ordinal 5.
    let kv = &mut rig
        .store
        .entry(SmolStr::new_static("tacsim"))
        .or_default()
        .kv;
    kv.retain(|(k, _)| k != &closing_key);
    kv.push((
        closing_key,
        json!({
            "format": 1,
            "site": "ahuna_port",
            "close": {
                "actor": owner.0,
                "ordinal": 5,
                "units": [],
                "known": known_rows,
                "minds": [],
            },
            "exits": [],
        }),
    ));
    let rig = reboot(rig, &[]);
    let owner_close = resolve_field_params(&rig, owner, "close", 3);
    let close_bytes =
        mush_core::mechanics::canonical_json_bytes_with_limit(&owner_close, 48 * 1024)
            .expect("owner close offer <= 48 KiB");
    assert!(close_bytes.len() <= 48 * 1024);
    assert_eq!(owner_close["closing"]["ahuna_port"]["ordinal"], 5);
}

/// **CRIT-19**: Amending and re-offering a `+bay/order` fee contract causes `verified_fee_payee`
/// to reject it so the facility operator NEVER signs the tampered contract and cancels the order.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn crit19_tampered_bay_order_fee_contract_is_refused_by_operator() {
    let mut rig = stocked();
    let (captain, _row, spare) = berthed_spare(&mut rig);
    let cannon = component_id(&unit(&rig, spare), "cannon");
    damage(
        &mut rig,
        spare,
        vec![UnitMutationOp::SetComponentCondition {
            component: cannon.clone(),
            condition: ConditionId::new("jammed").expect("jammed"),
        }],
    );

    let taken = run(
        &mut rig.tw,
        captain,
        &format!("+bay/order #{}=unjam", spare.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let order = order_of(&rig, spare);
    let contract_id = order.contract.expect("fee contract");
    // Locate the workshop operator's leased escort (still titled to the operator org) and attempt
    // the exact CRIT-19 exploit: amend the offered fee contract, attach a TitleTransfer
    // deliverable stealing the operator's asset, and re-offer!
    let leased_escort = rigs_of(&rig.tw, captain)[0];
    let operator_org = match rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(contract_id)))
        .expect("contract")
        .consent
        .parties
        .get(1)
    {
        Some(PrincipalId::Organization(org)) => *org,
        other => panic!("expected org counterparty: {other:?}"),
    };
    let operator_name = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|o| o.state().org(operator_org).map(|org| org.name.to_string()))
        .expect("operator org name");

    let _ = run(
        &mut rig.tw,
        captain,
        &format!("@contract/amend #{contract_id}"),
    );
    let delivered = run(
        &mut rig.tw,
        captain,
        &format!(
            "@contract/deliver #{contract_id}=title #{} org:{operator_name}->#{}",
            leased_escort.0, captain.0
        ),
    );
    let _ = run(
        &mut rig.tw,
        captain,
        &format!("@contract/offer #{contract_id}"),
    );
    let c = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(contract_id)).cloned())
        .expect("contract");
    assert_eq!(c.state, ContractState::Offered);
    assert!(
        !c.consent.terms.deliverables.is_empty(),
        "attacker injected deliverable into fee contract: {delivered}"
    );

    // Advance IC clock past `due_ic`: operator MUST refuse to sign and cancel the order!
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let finished = order_of(&rig, spare);
    assert_eq!(
        finished.status,
        OrderStatus::Cancelled,
        "tampered order must cancel, not complete: {finished:?}"
    );
    let after = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(contract_id)).cloned())
        .expect("contract");
    assert_eq!(
        after.state,
        ContractState::Cancelled,
        "operator must never sign a tampered fee contract (cancelled by close_returning)"
    );
    assert_eq!(
        title_of(&rig, leased_escort),
        Some(PrincipalId::Organization(operator_org)),
        "operator asset title was never stolen"
    );
    assert_eq!(
        tacsim::field_ops::assess_tacsim_unit(&unit(&rig, spare), None)
            .expect("assessed")
            .component_conditions[&cannon],
        ComponentCondition::Jammed,
        "work must not have landed after operator refused tampered fee contract"
    );
}

/// **HIGH-48 & HIGH-53**: `+bay/strip` counts crystallized divisible lots in the hold (HIGH-48)
/// AND accounts for stripped weapon `rounds` mass in the batch capacity check (HIGH-53).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn high48_and_high53_bay_strip_counts_crystallized_lots_and_stripped_weapon_rounds() {
    // Part 1 (HIGH-48): crystallized divisible lot fills the 86 t hold -> strip refused.
    {
        let mut rig = stocked();
        let (captain, row, spare) = berthed_spare(&mut rig);
        let hold = hold_vault_of(&rig.tw, &row);
        seed_hold(&mut rig, &row, "metals", HOLD_FULL_T, 99_500);
        crystallize(&mut rig, captain, hold, "metals", HOLD_FULL_T);
        assert_eq!(hold_pool(&mut rig.tw, hold, "metals"), 0);

        let refused = run(
            &mut rig.tw,
            captain,
            &format!("+bay/strip #{}=cannon", spare.0),
        );
        assert!(
            refused.contains("hold will not take") && refused.contains("The hold has"),
            "crystallized divisible lot must count in strip hold capacity: {refused}"
        );
        assert!(bay_record(&rig, spare).owed.is_none());
    }

    // Part 2 (HIGH-53): 86 t hold has room for the 2,400 kg cannon mount ALONE, but NOT for
    // cannon mount (2,400 kg) + loaded 105 mm rounds (21 kg each)!
    {
        let mut rig = stocked();
        let (captain, row, spare) = berthed_spare(&mut rig);
        let cannon = component_id(&unit(&rig, spare), "cannon");
        let (loaded_rounds, _) = magazine(&unit(&rig, spare), &cannon);
        assert!(loaded_rounds > 0, "escort cannon starts loaded");
        // Fill 83 t of structural_alloys (83,000 kg) + 25 rounds of ammo_he_105 (525 kg)
        // = 83,525 kg -> 2,475 kg free!
        // Cannon alone = 2,400 kg (fits!); cannon + loaded rounds >= 2,400 + 21*loaded > 2,475 kg!
        seed_hold(&mut rig, &row, "structural_alloys", 83, 99_510);
        seed_hold(&mut rig, &row, "ammo_he_105", 25, 99_511);

        let refused = run(
            &mut rig.tw,
            captain,
            &format!("+bay/strip #{}=cannon", spare.0),
        );
        assert!(
            refused.contains("hold will not take") && refused.contains("The hold has"),
            "stripped weapon rounds mass must be included in hold capacity check: {refused}"
        );
        assert!(bay_record(&rig, spare).owed.is_none());

        // Emptying the cannon's magazine leaves only the 2,400 kg part (<= 2,475 kg free), so
        // stripping now succeeds!
        damage(&mut rig, spare, vec![empty_magazine(&cannon)]);
        let ok = run(
            &mut rig.tw,
            captain,
            &format!("+bay/strip #{}=cannon", spare.0),
        );
        assert!(
            ok.contains("comes off"),
            "empty cannon fits 2,475 kg free room: {ok}"
        );
        for _ in 0..8 {
            if bay_record(&rig, spare).owed.is_none() {
                break;
            }
            let (_, queued) = fire(&mut rig);
            land_quietly(&mut rig, &queued);
        }
        assert!(bay_record(&rig, spare).owed.is_none());
    }
}

/// **HIGH-52**: `+bay #<id>` and `+bay/quote #<id>` refuse remote unprivileged actors with
/// `NO_RIGHT` (leaking zero doll/condition/ammo/location intel), while permitting co-located
/// observers, right holders, and staff.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn high52_bay_inspect_blocks_remote_strangers_and_allows_colocated_right_or_staff() {
    let mut rig = stocked();
    let (captain, row, spare) = berthed_spare(&mut rig);
    let stranger = pilot(&mut rig, "Spy", 100);

    // Put the spare back into the captain's carrier bay so it is NOT on the public workshop floor.
    let stow = run(&mut rig.tw, captain, &format!("+bay/stow #{}", spare.0));
    assert!(stow.contains("stowed"), "{stow}");

    // 1. Stranger remotely in Ceres exchange cannot inspect via `+bay` or `+bay/quote`.
    let market_room = station_room(&rig.tw, CERES, "room.c07_exchange");
    put(&mut rig, stranger, market_room, false);
    for cmd in [
        format!("+bay #{}", spare.0),
        format!("+bay/quote #{}", spare.0),
    ] {
        let out = run(&mut rig.tw, stranger, &cmd);
        assert!(
            out.contains(tacsim::port::bay_work::NO_RIGHT),
            "{cmd} must refuse remote stranger with NO_RIGHT: {out}"
        );
        assert!(
            !out.contains("Condition:") && !out.contains("rounds") && !out.contains("cannon"),
            "{cmd} leaked tactical doll or ammo intel: {out}"
        );
    }

    // 2. Stranger in the public workshop room still cannot inspect a hull inside Captain's bay.
    let workshop = station_room(&rig.tw, CERES, C14);
    put(&mut rig, stranger, workshop, false);
    let out = run(&mut rig.tw, stranger, &format!("+bay #{}", spare.0));
    assert!(out.contains(tacsim::port::bay_work::NO_RIGHT), "{out}");

    // 3. Right holder (captain) standing remotely CAN inspect.
    put(&mut rig, captain, market_room, false);
    let own = run(&mut rig.tw, captain, &format!("+bay #{}", spare.0));
    assert!(
        own.contains("Condition:"),
        "right holder can inspect: {own}"
    );

    // 4. Staff (`GOD`) standing remotely CAN inspect.
    let staff_out = run(&mut rig.tw, GOD, &format!("+bay #{}", spare.0));
    assert!(
        staff_out.contains("Condition:"),
        "staff can inspect: {staff_out}"
    );

    // 5. Co-located stranger (standing in the same room as the hull) CAN inspect.
    put(&mut rig, stranger, bay_room(&row), false);
    let colocated = run(&mut rig.tw, stranger, &format!("+bay #{}", spare.0));
    assert!(
        colocated.contains("Condition:"),
        "co-located observer can inspect: {colocated}"
    );
}
