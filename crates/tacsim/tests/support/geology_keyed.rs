//! **The keyed geology rig** (doc 20 cut SM.1's helpers, moved here by cut SM.2 so the site
//! suites share them): the fixed test keys, doc 20 §3.2's Ceres and Vesta tables as test data
//! (the lane's own tables equal them — `tests/sites.rs` asserts it), and the production roster
//! booted with a key handed in before the module boot pass, as `build_engine` does.

#![allow(dead_code)]

use std::collections::HashMap;

use mush_core::{GeologyMac, GeologySecret, ModuleBootData};
use mush_economy::EconomyConfig;
use mush_testkit::TestWorld;
use smol_str::SmolStr;
use tacsim::TacsimConfig;
use tacsim::surface::GeologyBoot;

/// The fixed test key (exactly 32 bytes; version 1).
pub const KEY_1: [u8; 32] = *b"rustmush-sm1-geology-test-key-01";
/// A second key.
pub const KEY_2: [u8; 32] = *b"rustmush-sm1-geology-test-key-02";

pub const CERES_RADIUS_KM: u32 = 470;
pub const VESTA_RADIUS_KM: u32 = 263;

/// Doc 20 §3.2's Ceres sentences as a table (test data; since doc 20 cut SM.2 the lane's own
/// `ceres` table equals it, `tests/sites.rs` `sm2_the_lane_tables_are_section_3_2s`).
pub const CERES_TABLE: &str = r#"{
  "province": {
    "metals": { "lean": 600, "fair": 400 },
    "volatiles": { "lean": 50, "fair": 500, "rich": 450 },
    "carbon_feedstock": { "lean": 1000 },
    "strategic_elements": { "barren": 950, "lean": 50 }
  },
  "field": {
    "barren": { "barren": 1000 },
    "lean": { "barren": 250, "lean": 550, "fair": 200 },
    "fair": { "barren": 100, "lean": 300, "fair": 450, "rich": 150 },
    "rich": { "lean": 150, "fair": 450, "rich": 400 }
  },
  "pocket": {
    "lean": { "lean": 700, "fair": 250, "rich": 50 },
    "fair": { "lean": 350, "fair": 500, "rich": 150 },
    "rich": { "lean": 150, "fair": 450, "rich": 400 }
  }
}"#;

/// Doc 20 §3.2's Vesta sentences as a table (test data).
pub const VESTA_TABLE: &str = r#"{
  "province": {
    "metals": { "fair": 550, "rich": 450 },
    "strategic_elements": { "lean": 600, "fair": 400 },
    "volatiles": { "lean": 1000 },
    "carbon_feedstock": { "barren": 1000 }
  },
  "field": {
    "barren": { "barren": 1000 },
    "lean": { "barren": 250, "lean": 550, "fair": 200 },
    "fair": { "barren": 100, "lean": 300, "fair": 450, "rich": 150 },
    "rich": { "lean": 150, "fair": 450, "rich": 400 }
  },
  "pocket": {
    "lean": { "lean": 700, "fair": 250, "rich": 50 },
    "fair": { "lean": 350, "fair": 500, "rich": 150 },
    "rich": { "lean": 150, "fair": 450, "rich": 400 }
  }
}"#;

pub fn secret(key: [u8; 32]) -> GeologySecret {
    GeologySecret::new(1, key).expect("a test key")
}

pub fn mac(key: [u8; 32]) -> GeologyMac {
    secret(key).mac()
}

pub fn config_with(enabled: bool) -> TacsimConfig {
    TacsimConfig {
        enabled,
        ..TacsimConfig::default()
    }
}

/// How a test roster is composed (`tests/hulls.rs`'s, minimal, with the layer's flag).
#[derive(Clone)]
pub struct Roster {
    /// Whether the pilot sheet composes (production does).
    sheet: bool,
    /// `[tacsim].enabled`.
    enabled: bool,
}

impl Roster {
    /// The production roster, the sheet included, the layer on or off.
    pub fn production_with(enabled: bool) -> Roster {
        Roster {
            sheet: true,
            enabled,
        }
    }

    pub fn modules(&self) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        let cfg = config_with(self.enabled);
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        if self.sheet {
            mods.extend(comp::sheet_modules(&onboarding, &game));
        }
        mods.push(comp::tacsim_module_wired(cfg, true, inventory));
        mods.push(comp::content_module_with(true, true).expect("the content module composes"));
        mods
    }
}

/// No module write was refused for its namespace.
pub fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
}

/// Every engine that needs a key — or a boot map, or the layer off — is built here: compose,
/// hand the key in, then run the module boot pass (`build_engine`'s order).
pub fn boot_keyed(
    boot: Option<GeologyBoot>,
    store: HashMap<SmolStr, ModuleBootData>,
    enabled: bool,
) -> TestWorld {
    boot_keyed_world(mush_testkit::boot_world().0, boot, store, enabled)
}

/// A kept store must be paired with its kept objects, including relay hardware.
pub fn reboot_keyed(
    tw: TestWorld,
    boot: Option<GeologyBoot>,
    store: HashMap<SmolStr, ModuleBootData>,
    enabled: bool,
) -> TestWorld {
    let mut world = tw.into_engine().world;
    let connected: Vec<_> = world
        .iter()
        .map(|o| o.id)
        .filter(|id| world.is_connected(*id))
        .collect();
    for id in connected {
        world.set_connected(id, false);
    }
    boot_keyed_world(world, boot, store, enabled)
}

fn boot_keyed_world(
    world: mush_core::World,
    boot: Option<GeologyBoot>,
    store: HashMap<SmolStr, ModuleBootData>,
    enabled: bool,
) -> TestWorld {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(
        world,
        registry,
        Roster::production_with(enabled).modules(),
    )
    .expect("the roster composes");
    if let Some(boot) = boot {
        assert!(tacsim::surface::install_geology(&mut engine.world, boot));
    }
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    engine
        .modules_loaded(store, &mut out, &mut fx)
        .expect("the world boots");
    let tw = TestWorld::from_engine(engine);
    no_refused_writes(&tw);
    tw
}
