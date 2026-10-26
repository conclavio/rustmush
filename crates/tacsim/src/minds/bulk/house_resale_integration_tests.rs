//! A separate actual transferable House purchase. Existing nontransferable
//! House stories retain their policy and assertions unchanged.
use super::*;
const START: u128 = 65_000_000;
pub(super) fn exercise(registry: &Fixture, snapshot: &Durable) -> TestResult {
    let birth = one(rows(&snapshot.disk, "roll", "roll_mind_initial_filing"))?;
    let owner: LicenceId =
        serde_json::from_value(birth.get("licence").cloned().ok_or("owner licence")?)?;
    // 17g C1a: the paid resale writer repeats the House buyer admission. The
    // probe (`resale_routes::AdmissionProbe`) stands in for the launch's
    // Coprates admission: it refuses only the pair the route arms, and records
    // every pair the fence asks about.
    let probe = Arc::new(resale_routes::AdmissionProbe::default());
    let admission = || -> Admission { Some(probe.clone()) };
    let mut w = boot_resale_options(registry, snapshot, true, true, admission())?;
    command(&mut w, &format!("@account/open #{}=cr", BUYER.0))?;
    let payer = wallet(&w, EconomicOwner::Object(BUYER), None)?;
    let faucet = wallet(
        &w,
        EconomicOwner::System(SystemPrincipalId::new("faucet").ok_or("faucet")?),
        None,
    )?;
    command(&mut w, &format!("@econ/mint {faucet}={payer} 2000"))?;
    perform(
        &mut w,
        "list_house_mind",
        GOD,
        START,
        json!({"owner_licence":owner.get().to_string()}),
    )?;
    perform(
        &mut w,
        "quote_house_mind",
        BUYER,
        START + 1,
        json!({"listing":token(START)}),
    )?;
    perform(
        &mut w,
        "purchase_house_mind",
        BUYER,
        START + 2,
        json!({"quote":token(START+1)}),
    )?;
    perform(
        &mut w,
        "settle_house_mind",
        BUYER,
        START + 3,
        json!({"operation":token(START+2)}),
    )?;
    let a = allocation(&w, START + 2)?;
    check!(matches!(a.phase(), PurchasePhase::Granted { .. }));
    let actual = term(&w, a.licence())?;
    check!(actual.terms().transferable());
    // Curator departure does not turn the House into a personal seller.
    command(
        &mut w,
        &format!("@house-purchase-curator-test {}", a.issuer().scope().0),
    )?;
    let granted = saved(&w);
    drop(w);
    resale_routes::exercise(
        &registry.accounts,
        &granted,
        a.licence(),
        BUYER,
        true,
        "faucet",
        START + 100,
        Some((probe.as_ref(), a.issuer())),
        |saved| boot_resale_options(registry, saved, true, true, admission()),
    )
}
