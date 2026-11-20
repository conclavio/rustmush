//! Tier 2A security regression tests for `mush-market`:
//! - Checked arithmetic on `price * qty`, maker spread scaling, and vendor `buy_permille`
//!   scaling (no silent `i64` saturation value collapse).
//! - `@market/vendor` existence oracle unification (`stock`, `create`, `withdraw`).

use std::sync::Arc;

use mush_core::{GOD, LIMBO, ObjId, ObjKind};
use mush_economy::{AccountId, EconomyStore, ExternalRef, FactProvider, PartyCap, VaultId};
use mush_inventory::{
    EndpointRef, Grade, InventoryFactProvider, InventoryFamilies, InventoryHandleCell,
    InventoryHandles, InventoryModule, LotId,
};
use mush_market::{
    CatalogEntry, CatalogSource, ListingAcceptance, MarketFactProvider, MarketStore,
    NO_SUCH_LISTING, NO_SUCH_VENDOR,
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
