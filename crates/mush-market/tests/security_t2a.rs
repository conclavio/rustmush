//! Tier 2A & Tier 2B security regression tests for `mush-market`:
//! - Checked arithmetic on `price * qty`, maker spread scaling, and vendor `buy_permille`
//!   scaling (no silent `i64` saturation value collapse).
//! - `@market/vendor` existence oracle unification (`stock`, `create`, `withdraw`).
//! - P2-MOD-7: `retire_vendor` (including `boot::retire_closed_orgs`) & `load_pack` preserve
//!   durable `WithdrawnHold` release auth and release dropped bid escrow.
//! - P2-MOD-29: `WithdrawnHold` release legs cannot be starved behind >64 live/lapsed/withdrawn
//!   holds, and `withdraw_own` enforces `may_take` and queue prechecks before mutation.
//! - P2-MOD-71: boot-healed settlement notifies `ProceedsPolicy` for personal lot asks with
//!   cuts/basis and private (`MarketVisibility::Hidden`) lot sales.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mush_core::{
    AuthoritativeSubmit, CommitToken, GOD, IdempotencyKey, IdempotencyScope, LIMBO, ObjId, ObjKind,
    OfferExecutionContext, OfferSubmission, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{
    AccountId, Amount, ContractId, ContractState, EconomyStore, ExternalRef, FactProvider,
    PartyCap, VaultId,
};
use mush_inventory::{
    EndpointRef, Grade, InventoryFactProvider, InventoryFamilies, InventoryHandleCell,
    InventoryHandles, InventoryModule, LotId,
};
use mush_market::{
    CatalogEntry, CatalogSource, ListingAcceptance, ListingId, MarketFactProvider, MarketStore,
    NO_SUCH_LISTING, NO_SUCH_VENDOR, PrivateSale, Proceeds, ProceedsCut, ProceedsPolicy,
    ProceedsRefusal, QuoteId, SaleTerms, SettledSale,
};
use mush_org::{OrgId, PrincipalId};
use mush_testkit::TestWorld;
use mush_testkit::fixtures::{err, ok};
use serde_json::json;

const PACK: &str = "t2a.market";
const BIG_BASELINE: i64 = i64::MAX / 10; // ~9.22e17: old `saturating_mul(500) / 1000` collapsed this!

#[derive(Debug)]
struct T2aCatalog;

impl CatalogSource for T2aCatalog {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack != PACK {
            return None;
        }
        let entries = vec![
            json!({
                "catalog": { "ruleset": "t2a", "code": "iron" },
                "pack": PACK,
                "definition_version": 1,
                "name": "Iron",
                "baseline": 1000,
                "legality": "legal",
                "representation": "lot",
                "stackable": true,
                "tags": ["test"],
            }),
            json!({
                "catalog": { "ruleset": "t2a", "code": "relic" },
                "pack": PACK,
                "definition_version": 1,
                "name": "Relic",
                "baseline": BIG_BASELINE,
                "legality": "legal",
                "representation": "asset",
                "stackable": false,
                "tags": ["test"],
            }),
            json!({
                "catalog": { "ruleset": "t2a", "code": "token" },
                "pack": PACK,
                "definition_version": 1,
                "name": "Token",
                "baseline": 100,
                "legality": "legal",
                "representation": "instrument",
                "stackable": true,
                "tags": ["test"],
            }),
        ];
        Some(
            entries
                .into_iter()
                .map(|v| serde_json::from_value::<CatalogEntry>(v).map_err(|e| e.to_string()))
                .collect(),
        )
    }
}

struct Rig {
    tw: TestWorld,
    families: InventoryFamilies,
    vendor_id: u64,
    vendor_org: OrgId,
    vendor_vault: VaultId,
    officer: ObjId,
    outsider: ObjId,
    outsider_vault: VaultId,
    faucet: AccountId,
}

fn boot_rig() -> Rig {
    let cell: InventoryHandleCell = Arc::default();
    let facts: Vec<Arc<dyn FactProvider>> = vec![
        Arc::new(MarketFactProvider),
        Arc::new(InventoryFactProvider),
    ];
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new()
                .with_party_caps(vec![(
                    PrincipalId::Organization(OrgId(0)),
                    PartyCap::Accept,
                )])
                .with_observers(vec![Arc::new(mush_market::MarketObserver)])
                .with_dispute_guard(Arc::new(mush_market::SaleDisputeGuard))
                .with_standing_acceptance(Arc::new(ListingAcceptance))
                .with_facts(facts),
        ),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["market"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(
            mush_market::MarketModule::new()
                .with_inventory(Arc::clone(&cell))
                .with_catalog_sources(vec![Arc::new(T2aCatalog)]),
        ),
    ]);
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();

    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let faucet = tw
        .world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .unwrap()
        .id;

    let officer = tw
        .world_mut()
        .create(ObjKind::Player, "Officer", GOD, Some(LIMBO))
        .unwrap();
    let outsider = tw
        .world_mut()
        .create(ObjKind::Player, "Outsider", GOD, Some(LIMBO))
        .unwrap();
    let shop_box = tw
        .world_mut()
        .create(ObjKind::Thing, "ShopVault", GOD, Some(LIMBO))
        .unwrap();
    let secret_box = tw
        .world_mut()
        .create(ObjKind::Thing, "SecretVault", GOD, Some(LIMBO))
        .unwrap();

    ok(&mut tw, GOD, "@org/create ShopOrg");
    let vendor_org = OrgId(0);
    let root = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .unwrap()
        .state()
        .org(vendor_org)
        .unwrap()
        .root_scope;

    ok(&mut tw, GOD, "@account/open org:ShopOrg=cr");
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", officer.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", outsider.0));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create shop=container:#{} owner=org:ShopOrg scope=#{}",
            shop_box.0, root.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create secret=container:#{} owner=#{}",
            secret_box.0, outsider.0
        ),
    );
    let (vendor_vault, outsider_vault) = {
        let econ = tw.world().module_state::<EconomyStore>().unwrap();
        let snap = econ.snapshot();
        let vv = snap
            .vaults()
            .find(|v| v.owner == PrincipalId::Organization(vendor_org))
            .unwrap()
            .id;
        let ov = snap
            .vaults()
            .find(|v| v.owner == PrincipalId::Object(outsider))
            .unwrap()
            .id;
        (vv, ov)
    };

    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=ShopOrg economy.account.post", officer.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=ShopOrg vault.withdraw", officer.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=ShopOrg vault.view", officer.0),
    );

    ok(&mut tw, GOD, &format!("@market/catalog load {PACK}"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@market/vendor create {}= {} name=Shop buy=500",
            vendor_org.0, vendor_vault.0
        ),
    );
    let vendor_id = tw
        .world()
        .module_state::<MarketStore>()
        .unwrap()
        .vendors()
        .next()
        .unwrap()
        .id
        .0;

    Rig {
        tw,
        families,
        vendor_id,
        vendor_org,
        vendor_vault,
        officer,
        outsider,
        outsider_vault,
        faucet,
    }
}

fn mint_lot(rig: &mut Rig, endpoint: VaultId, commodity: &str, qty: u64, byte: u8) -> LotId {
    let cid = mush_core::CanonicalId::new(commodity).unwrap();
    let lot_id =
        mush_inventory::derive_lot_id(mush_core::RequestUid::from_bytes([byte; 16]), 0).unwrap();
    let prepared = {
        let mut txn = rig
            .tw
            .engine_mut()
            .action_transaction(&rig.families.pools)
            .unwrap();
        rig.families
            .mint_origin(
                &mut txn,
                EndpointRef(endpoint),
                &cid,
                qty,
                Grade::new(0).unwrap(),
                ExternalRef {
                    provider: "test".into(),
                    kind: "seed".into(),
                    id: format!("t2a-{byte}").into(),
                    revision: None,
                },
                lot_id,
            )
            .unwrap();
        txn.prepare().unwrap()
    };
    let (_, install) = prepared.into_parts();
    rig.tw
        .engine_mut()
        .install_prepared_action(install)
        .unwrap();
    lot_id
}

#[test]
fn market_quote_and_sell_reject_overflow_and_preserve_large_i64_precision() {
    let mut rig = boot_rig();

    // 1. Price * qty overflow on quote must refuse instead of saturating to i64::MAX.
    let huge_price = i64::MAX / 2;
    ok(
        &mut rig.tw,
        GOD,
        &format!(
            "@market/vendor stock {}=t2a:token price={huge_price} qty=10",
            rig.vendor_id
        ),
    );
    let listing_id = rig
        .tw
        .world()
        .module_state::<MarketStore>()
        .unwrap()
        .live_listings()
        .next()
        .unwrap()
        .id
        .0;
    let quote_err = err(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/quote {listing_id}=3"),
    );
    assert!(
        quote_err.contains("exceed"),
        "expected overflow refusal on price * qty, got: {quote_err}"
    );
    // Single unit fits in i64 and still quotes cleanly!
    ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/quote {listing_id}=1"),
    );

    // 2. Exact-lot listing whose price * qty overflows i64 must be refused at listing time.
    let vendor_vault = rig.vendor_vault;
    let lot = mint_lot(&mut rig, vendor_vault, "iron", 5, 11);
    let stock_err = err(
        &mut rig.tw,
        rig.officer,
        &format!(
            "@market/vendor stock {}=t2a:iron price={huge_price} qty=5 endpoint={} lot={lot}",
            rig.vendor_id, rig.vendor_vault.0
        ),
    );
    assert!(
        stock_err.contains("exceed"),
        "expected overflow refusal on exact-lot listing, got: {stock_err}"
    );

    // 3. `+market/sell` with large `baseline` (`BIG_BASELINE = i64::MAX / 10`) and
    //    `buy_permille = 500` must offer EXACTLY `BIG_BASELINE / 2` (NOT ~0.1% from
    //    `i64::saturating_mul(500) / 1000`!).
    let vendor_acct = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .accounts()
        .find(|a| a.owner == PrincipalId::Organization(rig.vendor_org))
        .unwrap()
        .id;
    let expected_offer = i64::try_from((i128::from(BIG_BASELINE) * 500) / 1_000).unwrap();
    ok(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {}={vendor_acct} {expected_offer}", rig.faucet),
    );
    let relic = rig
        .tw
        .world_mut()
        .create(ObjKind::Thing, "AncientRelic", GOD, Some(rig.outsider))
        .unwrap();
    ok(
        &mut rig.tw,
        GOD,
        &format!(
            "@asset/title #{}=#{} catalog=t2a:relic",
            relic.0, rig.outsider.0
        ),
    );
    ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/sell #{}=Shop", relic.0),
    );
    let contract = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .contracts()
        .max_by_key(|c| c.id.0)
        .expect("sale contract");
    assert_eq!(
        contract.consent.terms.consideration[0].amount.minor(),
        expected_offer,
        "vendor buy_permille scaling must compute exact 50% in i128 without i64 saturation collapse"
    );
}

#[test]
fn market_vendor_closes_lot_object_and_create_existence_oracles() {
    let mut rig = boot_rig();

    // Seed a real lot inside outsider's private vault.
    let outsider_vault = rig.outsider_vault;
    let secret_lot = mint_lot(&mut rig, outsider_vault, "iron", 7, 22);
    let secret_obj = rig
        .tw
        .world_mut()
        .create(ObjKind::Thing, "SecretObject", GOD, Some(LIMBO))
        .unwrap();

    // 1. Non-officer calling `@market/vendor stock` cannot distinguish existing vs. missing vendor.
    let s_missing = err(
        &mut rig.tw,
        rig.outsider,
        "@market/vendor stock 999=t2a:iron price=10",
    );
    let s_existing = err(
        &mut rig.tw,
        rig.outsider,
        &format!("@market/vendor stock {}=t2a:iron price=10", rig.vendor_id),
    );
    assert!(s_missing.contains(NO_SUCH_VENDOR), "{s_missing}");
    assert_eq!(
        s_missing, s_existing,
        "non-officer distinguished existing vendor from missing vendor"
    );

    // 2. Officer probing outsider's private vault with `endpoint=` + `lot=` must get the
    //    IDENTICAL `NO_SUCH_LISTING` refusal whether the lot/commodity/qty matches or not!
    let wrong_lot =
        mush_inventory::derive_lot_id(mush_core::RequestUid::from_bytes([99; 16]), 0).unwrap();
    let probe_match = err(
        &mut rig.tw,
        rig.officer,
        &format!(
            "@market/vendor stock {}=t2a:iron price=10 qty=7 endpoint={} lot={secret_lot}",
            rig.vendor_id, rig.outsider_vault.0
        ),
    );
    let probe_wrong_qty = err(
        &mut rig.tw,
        rig.officer,
        &format!(
            "@market/vendor stock {}=t2a:iron price=10 qty=99 endpoint={} lot={secret_lot}",
            rig.vendor_id, rig.outsider_vault.0
        ),
    );
    let probe_wrong_lot = err(
        &mut rig.tw,
        rig.officer,
        &format!(
            "@market/vendor stock {}=t2a:iron price=10 qty=7 endpoint={} lot={wrong_lot}",
            rig.vendor_id, rig.outsider_vault.0
        ),
    );
    let probe_missing_vault = err(
        &mut rig.tw,
        rig.officer,
        &format!(
            "@market/vendor stock {}=t2a:iron price=10 qty=7 endpoint=99999 lot={wrong_lot}",
            rig.vendor_id
        ),
    );
    assert!(probe_match.contains(NO_SUCH_LISTING), "{probe_match}");
    assert_eq!(
        probe_match, probe_wrong_qty,
        "lot= leaked exact quantity in stranger's vault"
    );
    assert_eq!(
        probe_match, probe_wrong_lot,
        "lot= leaked existence of lot in stranger's vault"
    );
    assert_eq!(
        probe_match, probe_missing_vault,
        "endpoint= distinguished stranger's vault from nonexistent vault"
    );

    // 3. Officer passing `object=` on `@market/vendor stock` must NOT distinguish existing
    //    objects from nonexistent object IDs.
    let vendor_vault = rig.vendor_vault;
    let own_lot = mint_lot(&mut rig, vendor_vault, "iron", 5, 33);
    let obj_real = err(
        &mut rig.tw,
        rig.officer,
        &format!(
            "@market/vendor stock {}=t2a:iron price=10 qty=5 endpoint={} lot={own_lot} object=#{}",
            rig.vendor_id, rig.vendor_vault.0, secret_obj.0
        ),
    );
    let obj_missing = err(
        &mut rig.tw,
        rig.officer,
        &format!(
            "@market/vendor stock {}=t2a:iron price=10 qty=5 endpoint={} lot={own_lot} object=#99999",
            rig.vendor_id, rig.vendor_vault.0
        ),
    );
    assert_eq!(
        obj_real, obj_missing,
        "object= on @market/vendor stock leaked object existence"
    );

    // 4. Non-staff calling `@market/vendor create` gets "Permission denied." before arg parsing.
    let c_bad = err(&mut rig.tw, rig.outsider, "@market/vendor create bogus");
    let c_num = err(&mut rig.tw, rig.outsider, "@market/vendor create 0=0");
    assert_eq!(c_bad, c_num);
    assert!(c_bad.contains("Permission denied."), "{c_bad}");

    // 5. Officer cannot withdraw a stranger's personal listing via `@market/vendor withdraw`.
    ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/post {secret_lot} price=10 market=default"),
    );
    let personal_listing = rig
        .tw
        .world()
        .module_state::<MarketStore>()
        .unwrap()
        .live_listings()
        .find(|l| l.is_personal())
        .expect("personal listing")
        .id
        .0;
    let w_personal = err(
        &mut rig.tw,
        rig.officer,
        &format!("@market/vendor withdraw {personal_listing}"),
    );
    let w_missing = err(&mut rig.tw, rig.officer, "@market/vendor withdraw 99999");
    assert!(w_missing.contains(NO_SUCH_LISTING), "{w_missing}");
    assert_eq!(w_personal, w_missing);
}

// ---- Tier 2B security regression tests (P2-MOD-7, P2-MOD-29, P2-MOD-71) -----

const T2B_PACK: &str = "t2b.market";
const T2B_T0: i64 = 1_700_000_000;

#[derive(Debug)]
struct SwitchableCatalog {
    include_copper: Arc<AtomicBool>,
}

impl CatalogSource for SwitchableCatalog {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack != T2B_PACK {
            return None;
        }
        let mut entries = vec![json!({
            "catalog": { "ruleset": "t2b", "code": "iron" },
            "pack": T2B_PACK,
            "definition_version": 1,
            "name": "Iron",
            "baseline": 100,
            "legality": "legal",
            "representation": "lot",
            "stackable": true,
            "tags": ["test"],
        })];
        if self.include_copper.load(Ordering::Relaxed) {
            entries.push(json!({
                "catalog": { "ruleset": "t2b", "code": "copper" },
                "pack": T2B_PACK,
                "definition_version": 1,
                "name": "Copper",
                "baseline": 100,
                "legality": "legal",
                "representation": "lot",
                "stackable": true,
                "tags": ["test"],
            }));
        }
        Some(
            entries
                .into_iter()
                .map(|v| serde_json::from_value::<CatalogEntry>(v).map_err(|e| e.to_string()))
                .collect(),
        )
    }
}

fn enable_offer_keys(tw: &mut TestWorld) {
    tw.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x55; 32]).expect("offer secret"),
            [],
        )
        .expect("offer key ring"),
    );
}

fn take_submit(tw: &mut TestWorld) -> AuthoritativeSubmit {
    *tw.engine_mut()
        .take_authoritative_submit()
        .expect("staged authoritative submission")
}

fn land_submit(tw: &mut TestWorld, submit: &AuthoritativeSubmit, token: u16) -> Result<(), String> {
    let mut bytes = [0u8; 16];
    bytes[..2].copy_from_slice(&token.to_le_bytes());
    let (action, resolved) = {
        let engine = tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: submit.actor },
                submit.actor,
                submit.subject.clone(),
            )
            .map_err(|e| e.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|e| e.to_string())?;
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
            request: RequestUid::from_bytes(bytes),
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
                IdempotencyKey::Request(RequestUid::from_bytes(bytes)),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|e| format!("resolve: {e:?}"))?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    let (prepared, _) = tw
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes(bytes),
        )
        .map_err(|e| format!("prepare: {e:?}"))?;
    tw.engine_mut()
        .install_prepared_action(prepared.into_parts().1)
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

fn land_next(tw: &mut TestWorld, token: u16) {
    let submit = take_submit(tw);
    land_submit(tw, &submit, token).expect("staged submission lands");
}

fn tick_at(tw: &mut TestWorld, now: i64) {
    tw.set_now(now);
    let _ = tw.fire_due(now * 1000);
}

fn mint_lot_id(
    tw: &mut TestWorld,
    families: &InventoryFamilies,
    endpoint: VaultId,
    commodity: &str,
    qty: u64,
    seq: u32,
) -> LotId {
    let cid = mush_core::CanonicalId::new(commodity).unwrap();
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&seq.to_le_bytes());
    let lot_id = mush_inventory::derive_lot_id(RequestUid::from_bytes(bytes), 0).unwrap();
    let prepared = {
        let mut txn = tw.engine_mut().action_transaction(&families.pools).unwrap();
        families
            .mint_origin(
                &mut txn,
                EndpointRef(endpoint),
                &cid,
                qty,
                Grade::new(0).unwrap(),
                ExternalRef {
                    provider: "test".into(),
                    kind: "seed".into(),
                    id: format!("t2b-{seq}").into(),
                    revision: None,
                },
                lot_id,
            )
            .unwrap();
        txn.prepare().unwrap()
    };
    let (_, install) = prepared.into_parts();
    tw.engine_mut().install_prepared_action(install).unwrap();
    lot_id
}

fn lot_is_held(tw: &TestWorld, families: &InventoryFamilies, lot: LotId) -> bool {
    tw.engine()
        .action_state()
        .get(&families.lot_holds, &lot)
        .expect("read lot hold")
        .is_some()
}

fn parse_listing_id(line: &str, prefix: &str) -> ListingId {
    let tail = line
        .split(prefix)
        .nth(1)
        .unwrap_or_else(|| panic!("expected `{prefix}` in `{line}`"));
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    ListingId(digits.parse().expect("listing id"))
}

fn account_of(tw: &TestWorld, owner: &PrincipalId) -> AccountId {
    tw.world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .accounts()
        .find(|a| &a.owner == owner)
        .expect("account")
        .id
}

fn held_amount(tw: &TestWorld, acct: AccountId) -> i64 {
    tw.world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .account(acct)
        .expect("account")
        .held_amount
        .minor()
}

/// **P2-MOD-7**: `retire_vendor` (including `boot::retire_closed_orgs`) and `load_pack`
/// must route hold-backed deletions through the shared withdrawal helper so exact-lot
/// `WithdrawnHold` release auth is durably persisted (never stranding exact lots) and
/// dropped personal bids immediately release their escrow.
#[test]
fn p2_mod_7_retire_vendor_and_load_pack_preserve_withdrawn_holds_and_release_bid_escrow() {
    let include_copper = Arc::new(AtomicBool::new(true));
    let cell: InventoryHandleCell = Arc::default();
    let facts: Vec<Arc<dyn FactProvider>> = vec![
        Arc::new(MarketFactProvider),
        Arc::new(InventoryFactProvider),
    ];
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new()
                .with_party_caps(vec![
                    (PrincipalId::Organization(OrgId(0)), PartyCap::Accept),
                    (PrincipalId::Organization(OrgId(1)), PartyCap::Accept),
                ])
                .with_observers(vec![Arc::new(mush_market::MarketObserver)])
                .with_dispute_guard(Arc::new(mush_market::SaleDisputeGuard))
                .with_standing_acceptance(Arc::new(ListingAcceptance))
                .with_facts(facts),
        ),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["market"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(
            mush_market::MarketModule::new()
                .with_inventory(Arc::clone(&cell))
                .with_catalog_sources(vec![Arc::new(SwitchableCatalog {
                    include_copper: Arc::clone(&include_copper),
                })]),
        ),
    ]);
    enable_offer_keys(&mut tw);
    tw.set_now(T2B_T0);
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();

    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let faucet = tw
        .world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .unwrap()
        .id;

    let staff_wiz = tw
        .world_mut()
        .create(ObjKind::Player, "StaffWiz", GOD, Some(LIMBO))
        .unwrap();
    tw.world_mut()
        .get_mut(staff_wiz)
        .unwrap()
        .flags
        .insert(mush_core::Flags::WIZARD);
    let officer = tw
        .world_mut()
        .create(ObjKind::Player, "Officer", GOD, Some(LIMBO))
        .unwrap();
    let trader = tw
        .world_mut()
        .create(ObjKind::Player, "Trader", GOD, Some(LIMBO))
        .unwrap();

    // Org 0 (ShopOne) for interactive `retire_vendor`, Org 1 (ShopTwo) for `boot::retire_closed_orgs`.
    ok(&mut tw, GOD, "@org/create ShopOne");
    ok(&mut tw, GOD, "@org/create ShopTwo");
    let org1 = OrgId(0);
    let org2 = OrgId(1);
    ok(&mut tw, GOD, "@account/open org:ShopOne=cr");
    ok(&mut tw, GOD, "@account/open org:ShopTwo=cr");
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", trader.0));
    let trader_acct = account_of(&tw, &PrincipalId::Object(trader));
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={trader_acct} 5000"),
    );

    let box1 = tw
        .world_mut()
        .create(ObjKind::Thing, "Box1", GOD, Some(LIMBO))
        .unwrap();
    let box2 = tw
        .world_mut()
        .create(ObjKind::Thing, "Box2", GOD, Some(LIMBO))
        .unwrap();
    let tbox = tw
        .world_mut()
        .create(ObjKind::Thing, "TraderBox", trader, Some(trader))
        .unwrap();
    let (root1, root2) = {
        let orgs = tw.world().module_state::<mush_org::OrgStore>().unwrap();
        (
            orgs.state().org(org1).unwrap().root_scope,
            orgs.state().org(org2).unwrap().root_scope,
        )
    };
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create v1=container:#{} owner=org:ShopOne scope=#{}",
            box1.0, root1.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create v2=container:#{} owner=org:ShopTwo scope=#{}",
            box2.0, root2.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@vault/create vt=container:#{} owner=#{}", tbox.0, trader.0),
    );
    let (v1, v2, vt) = {
        let econ = tw.world().module_state::<EconomyStore>().unwrap();
        let snap = econ.snapshot();
        (
            snap.vaults()
                .find(|v| v.owner == PrincipalId::Organization(org1))
                .unwrap()
                .id,
            snap.vaults()
                .find(|v| v.owner == PrincipalId::Organization(org2))
                .unwrap()
                .id,
            snap.vaults()
                .find(|v| v.owner == PrincipalId::Object(trader))
                .unwrap()
                .id,
        )
    };
    for org_name in ["ShopOne", "ShopTwo"] {
        for cap in ["economy.account.post", "vault.withdraw", "vault.view"] {
            ok(
                &mut tw,
                GOD,
                &format!("@org/grant #{}={org_name} {cap}", officer.0),
            );
        }
    }

    ok(&mut tw, GOD, &format!("@market/catalog load {T2B_PACK}"));
    ok(
        &mut tw,
        GOD,
        &format!("@market/vendor create {}={} name=ShopOne", org1.0, v1.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@market/vendor create {}={} name=ShopTwo", org2.0, v2.0),
    );
    let (vid1, vid2) = {
        let s = tw.world().module_state::<MarketStore>().unwrap();
        let mut ids: Vec<u64> = s.vendors().map(|v| v.id.0).collect();
        ids.sort_unstable();
        (ids[0], ids[1])
    };

    // 1. `retire_vendor` with TWO exact-lot listings: both persist durable `WithdrawnHold`,
    //    at most one release is staged (no double-submit voiding), and both release cleanly
    //    even across a reboot!
    let lot_a = mint_lot_id(&mut tw, &families, v1, "iron", 5, 1);
    let lot_b = mint_lot_id(&mut tw, &families, v1, "iron", 5, 2);
    let s1 = ok(
        &mut tw,
        officer,
        &format!(
            "@market/vendor stock {vid1}=t2b:iron price=10 qty=5 endpoint={} lot={lot_a}",
            v1.0
        ),
    );
    let l1 = parse_listing_id(&s1, "Listing #");
    land_next(&mut tw, 1);
    let s2 = ok(
        &mut tw,
        officer,
        &format!(
            "@market/vendor stock {vid1}=t2b:iron price=12 qty=5 endpoint={} lot={lot_b}",
            v1.0
        ),
    );
    let l2 = parse_listing_id(&s2, "Listing #");
    land_next(&mut tw, 2);
    assert!(lot_is_held(&tw, &families, lot_a));
    assert!(lot_is_held(&tw, &families, lot_b));

    // Non-GOD staff Wizard retires vendor 1: stages at most 1 release (not voided!),
    // and persists WithdrawnHold for BOTH l1 and l2.
    ok(&mut tw, staff_wiz, &format!("@market/vendor retire {vid1}"));
    let staged_one = take_submit(&mut tw);
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "retire_vendor must stage at most one authoritative release per actor command"
    );
    {
        let store = tw.world().module_state::<MarketStore>().unwrap();
        assert!(store.listing(l1).is_none());
        assert!(store.listing(l2).is_none());
        assert!(
            store.withdrawn_hold(l1).is_some() && store.withdrawn_hold(l2).is_some(),
            "retire_vendor must persist WithdrawnHold for every hold-backed listing"
        );
    }
    // Land the one staged release (frees lot_a), then retry the second across a reboot (frees lot_b).
    land_submit(&mut tw, &staged_one, 3).expect("staged release from retire_vendor lands");
    assert!(!lot_is_held(&tw, &families, lot_a));
    assert!(lot_is_held(&tw, &families, lot_b));

    // 2. `boot::retire_closed_orgs`: vendor 2 has an exact-lot listing, its org is closed,
    //    and reboot runs `retire_closed_orgs` as GOD -> persists WithdrawnHold without
    //    staging a GOD submission, and officer releases the exact-lot hold post-boot!
    let lot_c = mint_lot_id(&mut tw, &families, v2, "iron", 5, 3);
    let s3 = ok(
        &mut tw,
        officer,
        &format!(
            "@market/vendor stock {vid2}=t2b:iron price=15 qty=5 endpoint={} lot={lot_c}",
            v2.0
        ),
    );
    let l3 = parse_listing_id(&s3, "Listing #");
    land_next(&mut tw, 4);
    assert!(lot_is_held(&tw, &families, lot_c));
    ok(&mut tw, GOD, "@org/status ShopTwo=closed");

    let boot = mush_testkit::fold_module_ops(tw.take_module_ops());
    tw.modules_loaded(boot);
    assert!(
        tw.engine_mut().take_authoritative_submit().is_none(),
        "boot::retire_closed_orgs must not stage authoritative submissions as GOD"
    );
    {
        let store = tw.world().module_state::<MarketStore>().unwrap();
        assert!(store.listing(l3).is_none());
        assert!(
            store.withdrawn_hold(l2).is_some(),
            "l2 WithdrawnHold survives reboot"
        );
        assert!(
            store.withdrawn_hold(l3).is_some(),
            "boot::retire_closed_orgs must persist WithdrawnHold for closed org's exact-lot listing"
        );
    }
    // Re-open ShopTwo or have staff Wizard release both l2 and l3 via `@market/vendor withdraw`.
    ok(
        &mut tw,
        staff_wiz,
        &format!("@market/vendor withdraw {}", l2.0),
    );
    land_next(&mut tw, 5);
    assert!(
        !lot_is_held(&tw, &families, lot_b),
        "lot_b released cleanly"
    );
    ok(
        &mut tw,
        staff_wiz,
        &format!("@market/vendor withdraw {}", l3.0),
    );
    land_next(&mut tw, 6);
    assert!(
        !lot_is_held(&tw, &families, lot_c),
        "lot_c released cleanly"
    );

    // 3. `load_pack` dropping `t2b:copper`: trader has BOTH a personal exact-lot ask AND
    //    a personal bid (with escrowed funds) for copper.
    //    Note: keep one live vendor in venue `default` so personal listings have a currency.
    ok(&mut tw, GOD, "@org/create ShopThree");
    let org3 = OrgId(2);
    ok(&mut tw, GOD, "@account/open org:ShopThree=cr");
    let box3 = tw
        .world_mut()
        .create(ObjKind::Thing, "Box3", GOD, Some(LIMBO))
        .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@vault/create v3=container:#{} owner=org:ShopThree", box3.0),
    );
    let v3 = tw
        .world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .vaults()
        .find(|v| v.owner == PrincipalId::Organization(org3))
        .unwrap()
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@market/vendor create {}={} name=ShopThree", org3.0, v3.0),
    );

    let lot_copper = mint_lot_id(&mut tw, &families, vt, "copper", 4, 4);
    let ask_line = ok(
        &mut tw,
        trader,
        &format!("+market/post {lot_copper} price=20 market=default"),
    );
    let ask_id = parse_listing_id(&ask_line, "as #");
    land_next(&mut tw, 7);
    assert!(lot_is_held(&tw, &families, lot_copper));

    let bid_line = ok(
        &mut tw,
        trader,
        &format!(
            "+market/bid t2b:copper qty=10 price=25 into={} market=default",
            vt.0
        ),
    );
    let bid_id = parse_listing_id(&bid_line, "Bid #");
    assert_eq!(held_amount(&tw, trader_acct), 250, "bid escrows 250 cr");

    // Reload pack with `t2b:copper` removed!
    include_copper.store(false, Ordering::Relaxed);
    ok(&mut tw, GOD, &format!("@market/catalog load {T2B_PACK}"));
    {
        let store = tw.world().module_state::<MarketStore>().unwrap();
        assert!(store.listing(ask_id).is_none());
        assert!(store.listing(bid_id).is_none());
        assert!(
            store.withdrawn_hold(ask_id).is_some(),
            "load_pack must persist WithdrawnHold for dropped exact-lot ask"
        );
    }
    assert_eq!(
        held_amount(&tw, trader_acct),
        0,
        "load_pack must immediately release escrow of dropped personal bid"
    );
    // Trader retries withdrawal of the dropped exact-lot ask and frees their exact lot!
    ok(&mut tw, trader, &format!("+market/withdraw {}", ask_id.0));
    land_next(&mut tw, 8);
    assert!(
        !lot_is_held(&tw, &families, lot_copper),
        "dropped exact-lot ask releases cleanly without stranding"
    );
    // Sweep prunes the Gone WithdrawnHold records.
    tick_at(&mut tw, T2B_T0 + 61);
    {
        let store = tw.world().module_state::<MarketStore>().unwrap();
        assert!(store.withdrawn_hold(ask_id).is_none());
        assert!(store.withdrawn_hold(l1).is_none());
        assert!(store.withdrawn_hold(l2).is_none());
        assert!(store.withdrawn_hold(l3).is_none());
    }
}

/// **P2-MOD-29**:
/// 1. `WithdrawnHold` release legs cannot be starved behind >64 (`MAX_LISTING_LEGS_PER_KIND`)
///    lower-ID lapsed hold-backed listings — both when freshly staged AND unstaged after reboot.
/// 2. `withdraw_own` enforces `may_take` (revoked `vault.withdraw` while `acts_for` holds) and
///    `authoritative_pending` (`POST_BUSY`) BEFORE deleting the listing or creating a `WithdrawnHold`.
#[test]
fn p2_mod_29_withdrawn_hold_not_starved_and_withdraw_own_checks_may_take_and_queue() {
    let mut rig = boot_rig();
    enable_offer_keys(&mut rig.tw);
    rig.tw.set_now(T2B_T0);

    // Create two more vendor orgs so we can hold >64 (70) lower-ID lapsed hold-backed
    // listings while staying within `MAX_LISTINGS_PER_VENDOR` (32).
    let mut vendor_slots: Vec<(u64, VaultId)> = vec![(rig.vendor_id, rig.vendor_vault)];
    for (org_name, box_name, vault_name) in [
        ("ShopTwo", "ShopTwoBox", "vtwo"),
        ("ShopThree", "ShopThreeBox", "vthree"),
    ] {
        ok(&mut rig.tw, GOD, &format!("@org/create {org_name}"));
        let (oid, root) = {
            let orgs = rig.tw.world().module_state::<mush_org::OrgStore>().unwrap();
            let o = orgs
                .state()
                .orgs()
                .find(|o| o.name == org_name)
                .expect("vendor org");
            (o.id, o.root_scope)
        };
        ok(
            &mut rig.tw,
            GOD,
            &format!("@account/open org:{org_name}=cr"),
        );
        let v_box = rig
            .tw
            .world_mut()
            .create(ObjKind::Thing, box_name, GOD, Some(LIMBO))
            .unwrap();
        ok(
            &mut rig.tw,
            GOD,
            &format!(
                "@vault/create {vault_name}=container:#{} owner=org:{org_name} scope=#{}",
                v_box.0, root.0
            ),
        );
        let vault_id = rig
            .tw
            .world()
            .module_state::<EconomyStore>()
            .unwrap()
            .snapshot()
            .vaults()
            .find(|v| v.owner == PrincipalId::Organization(oid))
            .expect("vendor vault")
            .id;
        for cap in ["economy.account.post", "vault.withdraw", "vault.view"] {
            ok(
                &mut rig.tw,
                GOD,
                &format!("@org/grant #{}={org_name} {cap}", rig.officer.0),
            );
        }
        ok(
            &mut rig.tw,
            GOD,
            &format!(
                "@market/vendor create {}={} name={org_name}",
                oid.0, vault_id.0
            ),
        );
        let vid = rig
            .tw
            .world()
            .module_state::<MarketStore>()
            .unwrap()
            .vendors()
            .find(|v| v.org == oid)
            .expect("vendor")
            .id
            .0;
        vendor_slots.push((vid, vault_id));
    }

    // Seed 70 lower-ID hold-backed vendor listings with `ttl=60`, landing each Reserve hold.
    let mut token: u16 = 1;
    for i in 0..70u32 {
        let (vid, vault_id) = vendor_slots[usize::try_from(i / 25).unwrap()];
        let lot = mint_lot_id(&mut rig.tw, &rig.families, vault_id, "iron", 1, i + 1);
        ok(
            &mut rig.tw,
            rig.officer,
            &format!(
                "@market/vendor stock {vid}=t2a:iron price=10 qty=1 endpoint={} lot={lot} ttl=60",
                vault_id.0
            ),
        );
        land_next(&mut rig.tw, token);
        token += 1;
    }

    // Advance past ttl=60 so all 70 lower-ID vendor listings are LAPSED hold-backed listings
    // (exceeding `MAX_LISTING_LEGS_PER_KIND = 64`!).
    tick_at(&mut rig.tw, T2B_T0 + 120);
    assert_eq!(
        rig.tw
            .world()
            .module_state::<MarketStore>()
            .unwrap()
            .listing_count(),
        70,
        "all 70 lapsed hold-backed listings remain kept while their holds stand"
    );

    // 1a. Freshly staged `WithdrawnHold` at higher ID #70 must NOT be starved behind the 70
    //     lower-ID lapsed listings.
    let victim_lot_1 = mint_lot_id(
        &mut rig.tw,
        &rig.families,
        rig.outsider_vault,
        "iron",
        5,
        200,
    );
    let post1 = ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/post {victim_lot_1} price=50 market=default"),
    );
    let id1 = parse_listing_id(&post1, "as #");
    land_next(&mut rig.tw, token);
    token += 1;
    assert!(lot_is_held(&rig.tw, &rig.families, victim_lot_1));

    ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/withdraw {}", id1.0),
    );
    land_next(&mut rig.tw, token);
    token += 1;
    assert!(
        !lot_is_held(&rig.tw, &rig.families, victim_lot_1),
        "staged WithdrawnHold release must succeed despite >64 lower-ID lapsed listings"
    );

    // 1b. Even UNSTAGED (after reboot wipes RAM `release_stage_order`), the partitioned
    //     release budget reserves half of `MAX_LISTING_LEGS_PER_KIND` for `withdrawn_holds`,
    //     so a captured release submission still lands cleanly!
    let victim_lot_2 = mint_lot_id(
        &mut rig.tw,
        &rig.families,
        rig.outsider_vault,
        "iron",
        5,
        201,
    );
    let post2 = ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/post {victim_lot_2} price=50 market=default"),
    );
    let id2 = parse_listing_id(&post2, "as #");
    land_next(&mut rig.tw, token);
    token += 1;

    ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/withdraw {}", id2.0),
    );
    let delayed_release = take_submit(&mut rig.tw);
    let boot = mush_testkit::fold_module_ops(rig.tw.take_module_ops());
    rig.tw.modules_loaded(boot);
    land_submit(&mut rig.tw, &delayed_release, token).expect(
        "unstaged WithdrawnHold release after reboot must not be starved behind 70 lapsed listings",
    );
    token += 1;
    assert!(!lot_is_held(&rig.tw, &rig.families, victim_lot_2));

    // 2a. `withdraw_own` enforces `may_take` BEFORE deleting a personal org ask:
    //     a second officer (`treasurer`) holds `economy.account.post` (`acts_for == true`)
    //     without `vault.withdraw` (`may_take == false`).
    let org_lot = mint_lot_id(&mut rig.tw, &rig.families, rig.vendor_vault, "iron", 5, 202);
    let org_post = ok(
        &mut rig.tw,
        rig.officer,
        &format!("+market/post {org_lot} price=50 market=default as=ShopOrg"),
    );
    let org_listing = parse_listing_id(&org_post, "as #");
    land_next(&mut rig.tw, token);
    token += 1;
    assert!(lot_is_held(&rig.tw, &rig.families, org_lot));

    let treasurer = rig
        .tw
        .world_mut()
        .create(ObjKind::Player, "Treasurer", GOD, Some(LIMBO))
        .unwrap();
    ok(
        &mut rig.tw,
        GOD,
        &format!("@org/grant #{}=ShopOrg economy.account.post", treasurer.0),
    );
    let revoked_err = err(
        &mut rig.tw,
        treasurer,
        &format!("+market/withdraw {}", org_listing.0),
    );
    assert!(
        revoked_err.contains(NO_SUCH_LISTING),
        "withdraw_own without may_take must return existence-hiding NO_SUCH_LISTING: {revoked_err}"
    );
    {
        let store = rig.tw.world().module_state::<MarketStore>().unwrap();
        assert!(
            store.listing(org_listing).is_some(),
            "refused withdraw_own must leave the listing intact"
        );
        assert!(
            store.withdrawn_hold(org_listing).is_none(),
            "refused withdraw_own must not create a WithdrawnHold"
        );
    }

    // 2b. `withdraw_own` enforces `authoritative_pending` (`POST_BUSY`) BEFORE deleting
    //     a hold-backed personal ask.
    let q_lot_a = mint_lot_id(
        &mut rig.tw,
        &rig.families,
        rig.outsider_vault,
        "iron",
        5,
        203,
    );
    let q_lot_b = mint_lot_id(
        &mut rig.tw,
        &rig.families,
        rig.outsider_vault,
        "iron",
        5,
        204,
    );
    let pa = ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/post {q_lot_a} price=50 market=default"),
    );
    let listing_a = parse_listing_id(&pa, "as #");
    land_next(&mut rig.tw, token);
    token += 1;

    // Stage a Reserve submission for `q_lot_b` and DO NOT pop it yet -> outsider has a
    // pending authoritative submission in the engine queue!
    ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/post {q_lot_b} price=50 market=default"),
    );
    let busy_err = err(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/withdraw {}", listing_a.0),
    );
    assert!(
        busy_err.contains(mush_market::seller::POST_BUSY),
        "withdraw_own while authoritative_pending must refuse with POST_BUSY: {busy_err}"
    );
    {
        let store = rig.tw.world().module_state::<MarketStore>().unwrap();
        assert!(
            store.listing(listing_a).is_some(),
            "POST_BUSY must leave listing_a intact"
        );
        assert!(
            store.withdrawn_hold(listing_a).is_none(),
            "POST_BUSY must not create a WithdrawnHold"
        );
    }
    // Drain/land the pending submit, then withdraw succeeds cleanly.
    land_next(&mut rig.tw, token);
    token += 1;
    ok(
        &mut rig.tw,
        rig.outsider,
        &format!("+market/withdraw {}", listing_a.0),
    );
    land_next(&mut rig.tw, token);
    assert!(!lot_is_held(&rig.tw, &rig.families, q_lot_a));
}

/// **P2-MOD-71**: Boot-healed settlement (`boot::reconcile_contracts` -> `fold_healed_sales`)
/// must notify `ProceedsPolicy` consistently with live settlement (`trade::settled_sale`) for:
/// 1. Public personal lot asks (`PaidKind::Hold`) with frozen cuts/basis -> `ProceedsPolicy::settled`.
/// 2. Private (`MarketVisibility::Hidden`) lot sales even with zero cuts/basis -> `ProceedsPolicy::private_sale`.
///
/// And a subsequent sweep must not double-notify.
#[test]
fn p2_mod_71_boot_healed_settlement_notifies_proceeds_policy_for_personal_asks_and_private_sales() {
    #[derive(Debug)]
    struct SpyProceeds {
        duty_acct: AccountId,
        settled_quotes: Mutex<Vec<QuoteId>>,
        private_quotes: Mutex<Vec<QuoteId>>,
    }

    impl ProceedsPolicy for SpyProceeds {
        fn proceeds(
            &self,
            _w: &mush_core::World,
            sale: &SaleTerms<'_>,
        ) -> Result<Proceeds, ProceedsRefusal> {
            // Public iron ask gets a cut + non-zero basis; private copper ask gets empty cuts
            // and zero basis so BOTH branches of `needs_proceeds_notification` are tested independently.
            if sale.commodity.as_str() == "iron" {
                Ok(Proceeds {
                    cuts: vec![ProceedsCut {
                        to: self.duty_acct,
                        amount: Amount::from_minor(10),
                        label: "duty".into(),
                    }],
                    basis: sale.gross,
                    line: None,
                    memo: None,
                })
            } else {
                Ok(Proceeds::default())
            }
        }

        fn settled(&self, _ctx: &mut mush_core::Ctx<'_>, sale: &SettledSale<'_>) {
            self.settled_quotes.lock().unwrap().push(sale.quote);
        }

        fn private_sale(&self, _ctx: &mut mush_core::Ctx<'_>, sale: &PrivateSale<'_>) {
            self.private_quotes.lock().unwrap().push(sale.quote);
        }
    }

    let include_copper = Arc::new(AtomicBool::new(true));
    // AccountId(1) will be ShopOrg's cr account (AccountId(0) is the system mint).
    // We verify its exact ID right after creation below.
    let spy = Arc::new(SpyProceeds {
        duty_acct: AccountId(1),
        settled_quotes: Mutex::new(Vec::new()),
        private_quotes: Mutex::new(Vec::new()),
    });
    let cell: InventoryHandleCell = Arc::default();
    let facts: Vec<Arc<dyn FactProvider>> = vec![
        Arc::new(MarketFactProvider),
        Arc::new(InventoryFactProvider),
    ];
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            mush_economy::EconomyModule::new()
                .with_party_caps(vec![(
                    PrincipalId::Organization(OrgId(0)),
                    PartyCap::Accept,
                )])
                .with_observers(vec![Arc::new(mush_market::MarketObserver)])
                .with_dispute_guard(Arc::new(mush_market::SaleDisputeGuard))
                .with_standing_acceptance(Arc::new(ListingAcceptance))
                .with_facts(facts),
        ),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["market"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(
            mush_market::MarketModule::new()
                .with_inventory(Arc::clone(&cell))
                .with_catalog_sources(vec![Arc::new(SwitchableCatalog { include_copper })])
                .with_proceeds_policy(Arc::clone(&spy) as Arc<dyn ProceedsPolicy>),
        ),
    ]);
    enable_offer_keys(&mut tw);
    tw.set_now(T2B_T0);
    let families = tw
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();

    ok(&mut tw, GOD, "@instrument/register cr=0");
    ok(&mut tw, GOD, "@account/system mint=cr");
    let faucet = tw
        .world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, PrincipalId::System(_)))
        .unwrap()
        .id;

    ok(&mut tw, GOD, "@org/create PortOrg");
    let org = OrgId(0);
    ok(&mut tw, GOD, "@account/open org:PortOrg=cr");
    let duty_acct = account_of(&tw, &PrincipalId::Organization(org));
    assert_eq!(duty_acct, spy.duty_acct, "duty account id matches spy");

    let shop_box = tw
        .world_mut()
        .create(ObjKind::Thing, "PortBox", GOD, Some(LIMBO))
        .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create pv=container:#{} owner=org:PortOrg",
            shop_box.0
        ),
    );
    let pv = tw
        .world()
        .module_state::<EconomyStore>()
        .unwrap()
        .snapshot()
        .vaults()
        .find(|v| v.owner == PrincipalId::Organization(org))
        .unwrap()
        .id;
    ok(&mut tw, GOD, &format!("@market/catalog load {T2B_PACK}"));
    ok(
        &mut tw,
        GOD,
        &format!("@market/vendor create {}={} name=Port", org.0, pv.0),
    );

    let seller = tw.spawn_player("Seller");
    let buyer = tw.spawn_player("Buyer");
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", seller.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=cr", buyer.0));
    let buyer_acct = account_of(&tw, &PrincipalId::Object(buyer));
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={buyer_acct} 10000"),
    );
    let sbox = tw
        .world_mut()
        .create(ObjKind::Thing, "SBox", seller, Some(seller))
        .unwrap();
    let bbox = tw
        .world_mut()
        .create(ObjKind::Thing, "BBox", buyer, Some(buyer))
        .unwrap();
    ok(
        &mut tw,
        GOD,
        &format!("@vault/create sv=container:#{} owner=#{}", sbox.0, seller.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@vault/create bv=container:#{} owner=#{}", bbox.0, buyer.0),
    );
    let (sv, bv) = {
        let snap = tw
            .world()
            .module_state::<EconomyStore>()
            .unwrap()
            .snapshot();
        (
            snap.vaults()
                .find(|v| v.owner == PrincipalId::Object(seller))
                .unwrap()
                .id,
            snap.vaults()
                .find(|v| v.owner == PrincipalId::Object(buyer))
                .unwrap()
                .id,
        )
    };

    // Helper to execute a full personal exact-lot sale and settle its contract.
    let mut now = T2B_T0;
    let mut token: u16 = 1;
    let mut run_sale = |tw: &mut TestWorld, commodity: &str, extra: &str, seq: u32| -> QuoteId {
        let lot = mint_lot_id(tw, &families, sv, commodity, 2, seq);
        let post_line = ok(
            tw,
            seller,
            &format!("+market/post {lot} price=100 market=default {extra}"),
        );
        let lid = parse_listing_id(&post_line, "as #");
        land_next(tw, token);
        token += 1;

        let qline = ok(tw, buyer, &format!("+market/quote {}=2", lid.0));
        let qid_digits: String = qline
            .split("Quote #")
            .nth(1)
            .unwrap_or_else(|| panic!("no Quote # in {qline}"))
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let qid = QuoteId(qid_digits.parse().unwrap());
        ok(tw, buyer, &format!("+market/accept {} to={}", qid.0, bv.0));
        land_next(tw, token);
        token += 1;

        let cid: ContractId = tw
            .world()
            .module_state::<MarketStore>()
            .unwrap()
            .quote(qid)
            .unwrap()
            .contract
            .unwrap();
        for _ in 0..12 {
            now += 61;
            tick_at(tw, now);
            let st = tw
                .world()
                .module_state::<EconomyStore>()
                .unwrap()
                .snapshot()
                .contract(cid)
                .map(|c| c.state);
            if matches!(st, Some(ContractState::Settled { .. })) {
                return qid;
            }
        }
        panic!("contract #{cid} did not settle");
    };

    // 1. Public personal exact-lot ask (iron -> has cuts + basis).
    let q_public = run_sale(&mut tw, "iron", "", 1);
    // 2. Private personal exact-lot ask (copper -> Hidden visibility, empty cuts, zero basis).
    let q_private = run_sale(&mut tw, "copper", "to=Buyer", 2);

    // Confirm live settlement notified both hooks.
    assert_eq!(*spy.settled_quotes.lock().unwrap(), vec![q_public]);
    assert_eq!(*spy.private_quotes.lock().unwrap(), vec![q_private]);

    // Simulate a crash between economy settling both contracts and market persisting `settled`
    // on its quote documents: clear `"settled"` on both quote docs in folded boot data and reboot.
    let mut boot = mush_testkit::fold_module_ops(tw.take_module_ops());
    let market_boot = boot.get_mut(mush_market::NS).expect("market boot data");
    for qid in [q_public, q_private] {
        let qkey = mush_market::quote_key(qid);
        let (_, val) = market_boot
            .kv
            .iter_mut()
            .find(|(k, _)| k == &qkey)
            .expect("quote doc in KV");
        val["settled"] = serde_json::Value::Null;
    }

    spy.settled_quotes.lock().unwrap().clear();
    spy.private_quotes.lock().unwrap().clear();

    // Reboot: `boot::reconcile_contracts` stamps both quotes and queues both in `healed_sales`.
    tw.modules_loaded(boot);
    {
        let m = tw.world().module_state::<MarketStore>().unwrap();
        let e = tw.world().module_state::<EconomyStore>().unwrap();
        let qp = m.quote(q_public).expect("q_public reloaded");
        let qv = m.quote(q_private).expect("q_private reloaded");
        let cp = e.snapshot().contract(qp.contract.unwrap()).map(|c| c.state);
        let cv = e.snapshot().contract(qv.contract.unwrap()).map(|c| c.state);
        assert!(
            qp.settled.is_some(),
            "q_public should be stamped by boot::reconcile_contracts; contract={cp:?}, qp={qp:?}"
        );
        assert!(
            qv.settled.is_some(),
            "q_private should be stamped by boot::reconcile_contracts; contract={cv:?}, qv={qv:?}"
        );
    }
    assert!(
        spy.settled_quotes.lock().unwrap().is_empty()
            && spy.private_quotes.lock().unwrap().is_empty(),
        "boot defers policy notifications until the first sweep tick (L4)"
    );

    // First sweep tick drains `healed_sales`: BOTH `settled` (for q_public) AND
    // `private_sale` (for q_private) must fire once!
    now += 61;
    tick_at(&mut tw, now);
    assert_eq!(
        *spy.settled_quotes.lock().unwrap(),
        vec![q_public],
        "boot-healed public personal lot ask with cuts/basis must notify ProceedsPolicy::settled"
    );
    assert_eq!(
        *spy.private_quotes.lock().unwrap(),
        vec![q_private],
        "boot-healed private lot sale must notify ProceedsPolicy::private_sale"
    );

    // Second sweep tick must NOT double-notify.
    now += 61;
    tick_at(&mut tw, now);
    assert_eq!(spy.settled_quotes.lock().unwrap().len(), 1);
    assert_eq!(spy.private_quotes.lock().unwrap().len(), 1);
}
