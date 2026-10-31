#![cfg(test)]

//! Doc 31 cut UD.4b: the bay part custody's shape and its guard. The held,
//! consumed and returned lots are exercised end to end by the Industry bay
//! provider's rig, which composes the kernel protocol these writes require.
use super::*;
use crate::{EndpointRef, Grade, InventoryHandleCell, InventoryModule};
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind, OwnerId};
use mush_core::{
    DefinitionRegistryBuilder, GOD, GameModule, ModError, ModuleBootData, ModuleCtx, ObjKind,
    StateFamilyHandle,
};
use mush_economy::{EconomyModule, EconomyStore, ExternalRef};
use mush_testkit::TestWorld;
use serde_json::json;
use std::sync::{Arc, OnceLock};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn action(id: &str) -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("industry").expect("an owner"),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").expect("a kind"),
                DefinitionId::new(id).expect("an id"),
                1,
                json!({"fixture": id}),
            )
            .expect("a descriptor"),
        )
        .expect("registers")
}
fn binding() -> BayPartBinding {
    BayPartBinding::new(
        action(ACCEPT_ACTION),
        action(INSTALL_ACTION),
        action(CANCEL_ACTION),
    )
    .expect("a binding")
}

#[test]
fn ud4b_the_binding_is_the_three_bay_actions() {
    binding();
    assert!(
        BayPartBinding::new(
            action(INSTALL_ACTION),
            action(ACCEPT_ACTION),
            action(CANCEL_ACTION)
        )
        .is_err()
    );
    assert!(
        BayPartBinding::new(
            action("accept_core_integration"),
            action(INSTALL_ACTION),
            action(CANCEL_ACTION)
        )
        .is_err()
    );
}

/// The custody record is the exact shape the kernel joins, held and after.
#[test]
fn ud4b_the_custody_is_the_kernels_shape() {
    let held = BayPartCustody {
        format: 1,
        order: 1,
        lot: LotId(5),
        stores: EndpointRef(mush_economy::VaultId(8)),
        accepted: CommitToken::from_bytes([1; 16]),
        state: BayPartState::Held,
        terminal: None,
    };
    held.validate().expect("valid");
    assert_eq!(
        serde_json::to_value(&held).expect("encodes"),
        json!({"format":1,"order":1,"lot":5,"stores":8,"accepted":CommitToken::from_bytes([1; 16]),"state":"held","terminal":null})
    );
    let consumed = BayPartCustody {
        state: BayPartState::Consumed,
        terminal: Some(CommitToken::from_bytes([3; 16])),
        ..held.clone()
    };
    consumed.validate().expect("valid");
    assert_eq!(
        serde_json::to_value(&consumed).expect("encodes")["state"],
        "consumed"
    );
    let stray = BayPartCustody {
        terminal: Some(CommitToken::from_bytes([3; 16])),
        ..held.clone()
    };
    assert!(stray.validate().is_err(), "a held part has no terminal");
    let mut missing = serde_json::to_value(&held).expect("encodes");
    missing
        .as_object_mut()
        .expect("an object")
        .remove("terminal");
    assert!(
        serde_json::from_value::<BayPartCustody>(missing).is_err(),
        "terminal is required, even when null"
    );
}

/// An Industry owner with one family of its own, for an Industry-owned txn.
struct Industry {
    handle: Arc<OnceLock<StateFamilyHandle<u64, u64>>>,
}
impl GameModule for Industry {
    fn name(&self) -> &'static str {
        "industry"
    }
    fn register(
        &mut self,
        reg: &mut mush_core::Registrar<'_>,
    ) -> std::result::Result<(), ModError> {
        let id = |s: &str| mush_core::CanonicalId::new(format!("industry.bay_fixture.{s}.v1"));
        let d = mush_core::StateFamilyDescriptor::new(
            mush_core::StateFamilyId::new("industry_bay_fixture")
                .map_err(|e| ModError::msg(e.to_string()))?,
            id("key").map_err(|e| ModError::msg(e.to_string()))?,
            id("value").map_err(|e| ModError::msg(e.to_string()))?,
            1,
            id("decode").map_err(|e| ModError::msg(e.to_string()))?,
            id("validate").map_err(|e| ModError::msg(e.to_string()))?,
            id("canonical").map_err(|e| ModError::msg(e.to_string()))?,
        )
        .map_err(|e| ModError::msg(e.to_string()))?;
        let h = reg.state_family(
            d,
            |k: &u64| Ok(k.to_string()),
            |v: &serde_json::Value| v.as_u64().ok_or_else(|| "a number".to_owned()),
            |_: &u64| Ok(()),
            |v: &u64| Ok(json!(v)),
        )?;
        let _ = self.handle.set(h);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        _: &mut ModuleCtx<'_>,
        _: ModuleBootData,
    ) -> std::result::Result<(), ModError> {
        Ok(())
    }
}

/// **The kernel guards the part families**: with the bay part custody
/// composed but no bay refit protocol, an Industry write that holds a part
/// is refused at preparation, and nothing is held.
#[test]
fn ud4b_a_part_hold_without_the_bay_protocol_is_refused() -> Result {
    let cell: InventoryHandleCell = Arc::default();
    let industry = Arc::default();
    let inventory = InventoryModule::new()
        .with_handle_cell(Arc::clone(&cell))
        .with_quantity_delegates(["industry"])
        .with_bay_parts(binding());
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(EconomyModule::new()),
        Box::new(inventory),
        Box::new(Industry {
            handle: Arc::clone(&industry),
        }),
    ];
    let mut w = TestWorld::with_modules(modules);
    w.world_mut().set_now(1000);
    let object = w
        .world_mut()
        .create(ObjKind::Thing, "Bay stores", GOD, None)?;
    w.run(
        GOD,
        &format!("@vault/create Bay stores=container:#{}", object.0),
    );
    let vault = w
        .world()
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .snapshot()
        .vaults()
        .find(|v| v.name.as_str() == "Bay stores")
        .ok_or("the vault")?
        .id;
    let stores = EndpointRef(vault);
    let f = cell.get().ok_or("families")?.clone();
    let lot = LotId(7);
    {
        let mut txn = w.engine_mut().action_transaction(&f.lots)?;
        f.mint_origin(
            &mut txn,
            stores,
            &mush_core::CanonicalId::new("test.part")?,
            1,
            Grade::new(3)?,
            ExternalRef {
                provider: "fixture".into(),
                kind: "ordinary_origin".into(),
                id: "7".into(),
                revision: Some(1),
            },
            lot,
        )?;
        let (_, install) = txn.prepare()?.into_parts();
        w.engine_mut().install_prepared_action(install)?;
    }
    let parts = f.bay_parts().ok_or("the bay parts")?.clone();
    let handle = industry.get().ok_or("the industry family")?.clone();
    let mut txn = w.engine_mut().action_transaction(&handle)?;
    parts.reserve(&mut txn, 1, lot, stores, CommitToken::from_bytes([1; 16]))?;
    let refused = txn.prepare().err().ok_or("the kernel refuses the write")?;
    if !refused.to_string().contains("bay refit") {
        return Err(format!("refused by the bay protocol, not: {refused}").into());
    }
    Ok(())
}
