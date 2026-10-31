#![cfg(test)]

//! Doc 31 cut UD.4b, on the shipped content: a launch deployment naming a
//! refit bay composes the bay through the shipped hardware composition, and
//! a line tank there is refitted to its laser variant — the cannon out of
//! the turret, a laser from the bay's stores in, the hull's heat pair added —
//! over the bay's IC hours, for its quoted fee.
use super::*;
use mush_inventory::LotId;

const PART: LotId = LotId(77);
const FEE: i64 = 4_000;

/// Provisioning: a bay of the Basin organization's factory scope, its
/// stores holding one laser, and a line tank in it.
fn bay(r: &mut Rig) -> Result {
    let agent = r.deployment.agents[1].actor;
    let scope = r.deployment.integrator.scope;
    let w = r.world_mut();
    let bay = w
        .world_mut()
        .create(ObjKind::Thing, "Basin refit bay", GOD, None)?;
    let stores = w
        .world_mut()
        .create(ObjKind::Thing, "Basin bay stores", GOD, Some(bay))?;
    for line in [
        format!(
            "@facility/open {}=2 {}",
            bay.0,
            mush_industry::bay_refit::BAY_CAPABILITY
        ),
        format!("@facility/owner {}=org:Basin", bay.0),
        format!("@asset/title #{}=org:Basin scope={}", bay.0, scope.0),
        format!(
            "@vault/create BayStores=container:#{} owner=org:Basin scope={}",
            stores.0, scope.0
        ),
        format!(
            "@org/grant #{}=Basin {} scope=#{}",
            agent.0,
            mush_economy::bay_refit::OPERATOR_CAPABILITY,
            scope.0
        ),
    ] {
        command(w, GOD, &line)?;
    }
    let vault = w
        .world()
        .module_state::<EconomyStore>()
        .ok_or("Economy owner")?
        .snapshot()
        .vaults()
        .find(|v| v.name.as_str() == "BayStores")
        .ok_or("the bay's stores")?
        .id;
    let families = w
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .ok_or("Industry handles")?
        .0
        .inventory
        .clone();
    let p = {
        let mut txn = w.engine_mut().action_transaction(&families.lots)?;
        families.mint_origin(
            &mut txn,
            mush_inventory::EndpointRef(vault),
            &mush_core::CanonicalId::new("tacsim.part.laser")?,
            1,
            Grade::new(9)?,
            mush_economy::ExternalRef {
                provider: "fixture".into(),
                kind: "bay_part".into(),
                id: "77".into(),
                revision: Some(1),
            },
            PART,
        )?;
        txn.prepare()?
    };
    let (writes, install) = p.into_parts();
    w.engine_mut().install_prepared_action(install)?;
    fold_module_ops_into(&mut r.boot, writes);
    r.deployment.bay = Some(super::super::RefitBay {
        provider: r.deployment.integrator.clone(),
        facility: bay,
        stores: vault,
        max_fee_minor: None,
    });
    Ok(())
}

/// A buyer's line tank in the bay.
fn tank(r: &mut Rig, at: ObjId) -> Result<ObjId> {
    let buyer = r.buyer;
    let hull =
        r.world_mut()
            .world_mut()
            .create(ObjKind::Thing, "Basin line tank", buyer, Some(at))?;
    command(
        r.world_mut(),
        GOD,
        &format!("@unit/create #{}=tacsim.unit.line_tank", hull.0),
    )?;
    command(
        r.world_mut(),
        GOD,
        &format!("@asset/title #{}=#{}", hull.0, buyer.0),
    )?;
    r.flush();
    Ok(hull)
}

#[tokio::test(flavor = "current_thread")]
async fn ud4b_a_shipped_line_tank_is_refitted_to_its_laser_in_a_bay() -> Result {
    let mut r = Rig::new_with(Vec::new(), bay)?;
    let site = r.deployment.bay.clone().ok_or("the composed bay")?;
    let buyer = r.buyer;
    let agent = r.deployment.agents[1].actor;
    r.world_mut().world_mut().move_to(buyer, site.facility)?;
    let hull = tank(&mut r, site.facility)?;
    let (before_balance, _) = r.money(r.payer)?;
    let (offer, _) = r.perform_direct(
        "industry",
        "offer_bay_refit",
        agent,
        json!({"requester":buyer,"hull":hull,"facility":site.facility,
            "target":"tacsim.unit.line_tank_laser","part":PART,"fee":FEE}),
    )?;
    let (_, accepted) = r.perform_direct(
        "industry",
        "accept_bay_refit",
        buyer,
        json!({"offer":offer.to_hex(),"payer":r.payer.0.to_string()}),
    )?;
    check_eq!(
        r.money(r.payer)?,
        (before_balance, FEE),
        "the quote is held"
    );
    let order =
        serde_json::from_value::<mush_industry::workorder::WorkOrderId>(accepted["order"].clone())?;
    r.progress("advance_bay_refit", 4)?;
    r.perform_direct(
        "industry",
        "install_bay_refit",
        buyer,
        json!({"order":order.to_string()}),
    )?;
    let unit = mush_tactics::module::unit_of(r.world().world(), hull).ok_or("the hull's unit")?;
    check_eq!(
        unit.schema().key().id.as_str(),
        "tacsim.unit.line_tank_laser"
    );
    check!(
        unit.cells().any(
            |(k, _)| matches!(k, mush_tactics::CellAddr::Section { cell, .. }
            if cell.as_str() == crate::definitions::unit::HEAT_CELL)
        ),
        "the laser variant's hull carries the heat pair"
    );
    check_eq!(
        r.money(r.payer)?,
        (before_balance - FEE, 0),
        "the quote is paid"
    );
    let returned = r
        .rows("inventory_lots")
        .into_iter()
        .find(|v| v["commodity"] == "tacsim.part.cannon")
        .ok_or("the cannon returned to the bay's stores")?;
    check_eq!(returned["endpoint"], json!(site.stores));
    check!(
        r.rows("inventory_lots")
            .iter()
            .all(|v| v["id"] != json!(PART)),
        "the laser part is consumed"
    );
    // The shipped composition's own boot audits the whole bay.
    r.reboot(CoreOriginMode::Active)?;
    let unit = mush_tactics::module::unit_of(r.world().world(), hull).ok_or("the hull's unit")?;
    check_eq!(
        unit.schema().key().id.as_str(),
        "tacsim.unit.line_tank_laser"
    );
    Ok(())
}
