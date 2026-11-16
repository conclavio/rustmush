//! These exercise authored content and the actual registered Tacsim pack owner.
//! They do not claim a manufacture, paid delivery or refit acceptance story.
use super::content::CLASSES;
use super::recipes::SOURCE;
use super::*;
use crate::{PackEntry, TacsimConfig, TacsimRuleset, TacsimStore};
use mush_core::CanonicalId;
use mush_industry::{
    RecipeAuthority, RecipeRef,
    core_production::{DieClass, integration::CoreRecipeAuthority, refit::RefitRecipeAuthority},
};
use serde_json::{Value, json};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn body() -> Result<Value> {
    Ok(serde_json::from_str(SOURCE)?)
}
fn sign(mut value: Value) -> Result<String> {
    value["manifest"]["digest"] = json!("0".repeat(64));
    let parsed = mush_content::parse_pack(&serde_json::to_string(&value)?)?;
    value["manifest"]["digest"] = json!(parsed.computed_digest()?.to_hex());
    Ok(serde_json::to_string(&value)?)
}

#[test]
fn shipped_hardware_pins_prices_quotas_and_bounded_hourly_schedules() -> Result {
    let hardware = PublishedHardware::shipped()?;
    check_eq!(
        hardware.pack().manifest().digest().to_hex(),
        // Doc 17h cut M8b: the `core_ungovern` recipe and `ungovern_minor` joined the pack.
        // Doc 48 unit PL.7: and the dies' chain bill.
        "404c9588062e4e96c34a7bb8b7f8bcc3a9ad5112878f02e1ad9cbd11cbf67e5d"
    );
    check_eq!(hardware.catalogue().monthly_dies(), [64, 16, 4]);
    check_eq!(hardware.catalogue().fitting_minor(), 10_000);
    // Doc 17h cut M8b C7: the ungoverning refit's fee, provisionally the fitting fee.
    check_eq!(hardware.catalogue().ungovern_minor(), 10_000);
    for (index, class) in CLASSES.into_iter().enumerate() {
        check_eq!(
            hardware.catalogue().finished_minor(class),
            [300_000, 800_000, 2_500_000][index]
        );
        check_eq!(
            hardware.catalogue().die_minor(class)?
                + hardware.catalogue().integration_minor(class)?,
            hardware.catalogue().finished_minor(class)
        );
        for (process, units) in [
            (HardwareProcess::Integrate, [24, 48, 96]),
            (HardwareProcess::Refit, [1, 2, 4]),
            (HardwareProcess::Ungovern, [1, 2, 4]),
        ] {
            let Some(PackEntry::HardwareRecipe(recipe)) = hardware
                .pack()
                .entry(&CanonicalId::new(process.recipe_id())?)
            else {
                return Err("recipe missing".into());
            };
            check_eq!(recipe.work_units(class), units[index]);
            check_eq!(recipe.process(), process);
        }
    }
    Ok(())
}

#[test]
fn hardware_catalogue_refuses_a_price_display_that_differs_from_money() -> Result {
    for (field, wrong) in [
        ("scale", json!(0)),
        ("instrument", json!("other.credit")),
        ("tank_minor", json!(9_999_999)),
        // Doc 48 unit PL.7: the prices are content, but bounded as the payment policy bounds
        // them.
        ("core_minor", json!([0, 800_000, 2_500_000])),
        ("core_minor", json!([300_000, 800_000, 10_000_000_001_i64])),
        ("integration_permille", json!(0)),
        ("integration_permille", json!(1000)),
        ("fitting_minor", json!(9_999)),
        ("monthly_dies", json!([64, 0, 4])),
        ("unexpected", json!(true)),
    ] {
        let mut value = body()?["entries"][CATALOGUE]["def"].clone();
        value[field] = wrong;
        check!(
            serde_json::from_value::<HardwareCatalogueDef>(value).is_err(),
            "{field}"
        );
    }
    Ok(())
}

/// **Doc 48 unit PL.7: the core prices and the dies' chain are content** (Q-48-27). The shipped
/// catalogue carries the v1 prices and bills a die one board and a tonne of strategic ore at
/// Class I, two boards and a tonne at Class II, four boards and two tonnes at Class III. A
/// catalogue at other prices loads and answers them, which the v1 table never pinned before; a
/// chain out of order, naming a good twice, more than four goods, zero units or the die's own
/// commodity is refused.
#[test]
fn pl7_the_core_prices_and_the_die_chain_are_content() -> Result {
    use mush_economy::core_integration::{CoreClass, CorePrices};
    let hardware = PublishedHardware::shipped()?;
    check_eq!(hardware.catalogue().prices(), CorePrices::V1);
    let chain = hardware.catalogue().chain()?;
    let bill = |lines: &[mush_industry::core_production::ChainLine]| {
        lines
            .iter()
            .map(|l| (l.commodity.as_str().to_owned(), l.units))
            .collect::<Vec<_>>()
    };
    let line = |good: &str, units: u64| (good.to_owned(), units);
    check_eq!(
        bill(&chain[0]),
        vec![line("instrument_boards", 1), line("strategic_ore", 1)]
    );
    check_eq!(
        bill(&chain[1]),
        vec![line("instrument_boards", 2), line("strategic_ore", 1)]
    );
    check_eq!(
        bill(&chain[2]),
        vec![line("instrument_boards", 4), line("strategic_ore", 2)]
    );
    // Every chain good is a pooled good of the shipped table.
    let goods = crate::definitions::goods::goods_table()?;
    for l in chain.iter().flatten() {
        check!(
            goods.good(&l.commodity).is_some(),
            "{} is a good",
            l.commodity
        );
        check!(
            !crate::port_tick::is_part(&l.commodity),
            "{} pools",
            l.commodity
        );
    }
    // Other prices load: the catalogue is the price source, not the v1 table.
    let mut value = body()?["entries"][CATALOGUE]["def"].clone();
    value["core_minor"] = json!([450_000, 900_000, 3_000_000]);
    value["integration_permille"] = json!(150);
    let priced = serde_json::from_value::<HardwareCatalogueDef>(value)?;
    check_eq!(priced.finished_minor(CoreClass::II), 900_000);
    check_eq!(priced.integration_minor(CoreClass::II)?, 135_000);
    check_eq!(priced.die_minor(CoreClass::II)?, 765_000);
    check_eq!(
        priced.prices(),
        CorePrices {
            finished_minor: [450_000, 900_000, 3_000_000],
            integration_permille: 150,
        }
    );
    let entry = |good: &str, units: u64| json!({"good": good, "units": units});
    for wrong in [
        json!([
            [entry("strategic_ore", 1), entry("instrument_boards", 1)],
            [],
            []
        ]),
        json!([
            [entry("strategic_ore", 1), entry("strategic_ore", 1)],
            [],
            []
        ]),
        json!([[entry("strategic_ore", 0)], [], []]),
        json!([[entry("strategic_ore", 1_001)], [], []]),
        json!([[entry("tacsim.electronics", 1)], [], []]),
        json!([
            [
                entry("a", 1),
                entry("b", 1),
                entry("c", 1),
                entry("d", 1),
                entry("e", 1)
            ],
            [],
            []
        ]),
        json!([[], []]),
    ] {
        let mut value = body()?["entries"][CATALOGUE]["def"].clone();
        value["die_chain"] = wrong.clone();
        check!(
            serde_json::from_value::<HardwareCatalogueDef>(value).is_err(),
            "{wrong}"
        );
    }
    Ok(())
}

#[test]
fn hardware_recipes_refuse_unbounded_work_commodity_fields_and_seed_placement() -> Result {
    for (field, wrong) in [
        ("work_units", json!([0, 48, 96])),
        ("work_units", json!([24, 48, 168])),
        ("label", json!(" ")),
        ("process", json!("mint")),
        ("inputs", json!([])),
    ] {
        let mut value = body()?["entries"]["core_integrate"]["def"].clone();
        value[field] = wrong;
        check!(
            serde_json::from_value::<HardwareRecipeDef>(value).is_err(),
            "{field}"
        );
    }
    let mut seed = body()?;
    seed["manifest"]["kind"] = json!("campaign_seed");
    check!(crate::load_pack(&sign(seed)?).is_err());
    Ok(())
}

#[test]
fn actual_pack_owner_publication_and_loaded_bytes_are_both_required() -> Result {
    let hardware = PublishedHardware::shipped()?;
    let config = TacsimConfig {
        enabled: true,
        packs: vec![crate::PackSource {
            name: "shipped governed hardware".into(),
            bytes: SOURCE.into(),
        }],
        ..TacsimConfig::default()
    };
    let mut tw =
        mush_testkit::TestWorld::with_modules(vec![Box::new(TacsimRuleset::with_config(config))]);
    let key = (CanonicalId::new(PACK)?, VERSION);
    let receipt = tw
        .world_mut()
        .module_state_mut::<TacsimStore>()
        .ok_or("store missing")?
        .published
        .remove(&key)
        .ok_or("actual owner did not publish")?;
    // The actual boot loaded this pack. Removing the publication must still
    // refuse it: decoding or registration never substitutes for publication.
    check!(CoreRecipeAuthority::resolve(&hardware, tw.world(), DieClass::II).is_none());
    check!(RefitRecipeAuthority::resolve(&hardware, tw.world(), DieClass::II).is_none());
    tw.world_mut()
        .module_state_mut::<TacsimStore>()
        .ok_or("store missing")?
        .published
        .insert(key.clone(), receipt);
    for class in DieClass::ALL {
        let integration = CoreRecipeAuthority::resolve(&hardware, tw.world(), class)
            .ok_or("integration unavailable")?;
        let refit = RefitRecipeAuthority::resolve(&hardware, tw.world(), class)
            .ok_or("refit unavailable")?;
        check_eq!(
            integration.definition(),
            hardware.definition(HardwareProcess::Integrate)
        );
        check_eq!(
            refit.definition(),
            hardware.definition(HardwareProcess::Refit)
        );
        check_eq!(
            CoreRecipeAuthority::published_digest(&hardware, tw.world(), integration.pack()),
            Some(integration.pack().digest())
        );
        check!(
            CoreRecipeAuthority::published_digest(&hardware, tw.world(), refit.pack()).is_none()
        );
        let crossed = RecipeRef::new(
            CanonicalId::new("other.pack")?,
            VERSION,
            CanonicalId::new("core_integrate")?,
            integration.pack().digest(),
        )?;
        check!(CoreRecipeAuthority::published_digest(&hardware, tw.world(), &crossed).is_none());
    }
    // Ordinary commodity manufacture has no view of these certified recipes.
    check!(
        crate::composition::industry_recipes::TacsimRecipeAuthority
            .catalog(tw.world())
            .is_empty()
    );
    tw.world_mut()
        .module_state_mut::<TacsimStore>()
        .ok_or("store missing")?
        .loaded
        .remove(&key);
    check!(CoreRecipeAuthority::resolve(&hardware, tw.world(), DieClass::I).is_none());
    check!(RefitRecipeAuthority::resolve(&hardware, tw.world(), DieClass::III).is_none());
    Ok(())
}
