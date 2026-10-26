//! 16b cut R-b: the one game's settlement currency (doc 11 §2, `tacsim.credit`)
//! is registered create-only by the pilot sheet's boot, the way the deleted WoD
//! ruleset registered `wod.usd`.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, GOD, GameModule, ModError, Registrar};
use mush_economy::{EconomyModule, EconomyStore, InstrumentId, InstrumentKind, SymbolPosition};
use mush_jobs::JobsModule;
use mush_testkit::TestWorld;
use tacsim::sheet::PilotRuleset;
use tacsim::{CREDIT, CREDIT_CODE, CREDIT_SCALE, GameEconomyConfig};

const SEED: u64 = 4242;

fn credit() -> InstrumentId {
    InstrumentId::new(CREDIT).expect("the credit's id is canonical")
}

/// The store's view of the credit, if the boot registered it.
fn credit_scale(tw: &TestWorld) -> Option<(String, u8)> {
    let store = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("the economy module owns its store");
    let snapshot = store.snapshot();
    snapshot
        .instrument(&credit())
        .map(|def| (def.code.to_string(), def.scale))
}

#[test]
fn the_sheet_registers_the_credit_at_boot() {
    // `TestWorld::with_modules` boots the roster, `on_world_loaded` included.
    let tw = TestWorld::with_modules(vec![
        Box::new(EconomyModule::new()),
        Box::new(JobsModule::new()),
        Box::new(PilotRuleset::with_seed(SEED).economy(GameEconomyConfig::default())),
    ]);
    let store = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("the economy module owns its store");
    let snapshot = store.snapshot();
    let def = snapshot
        .instrument(&credit())
        .expect("tacsim.credit is registered by the sheet's boot");
    assert_eq!(def.code.as_str(), CREDIT_CODE);
    assert_eq!(def.scale, CREDIT_SCALE);
    assert_eq!(def.kind, InstrumentKind::Currency);
    assert_eq!(def.ruleset.as_str(), "tacsim");
    let display = def
        .display
        .as_ref()
        .expect("the credit renders as `<n> cr`");
    assert_eq!(display.symbol.as_str(), CREDIT_CODE);
    assert_eq!(display.position, SymbolPosition::Suffix);
    // The boot wrote nothing it was not entitled to write.
    assert!(
        tw.world().refused_namespace_writes().is_empty(),
        "the sheet's boot crossed a namespace it never declared: {:?}",
        tw.world().refused_namespace_writes()
    );
}

/// A test module whose one verb calls [`tacsim::economy::ensure_instrument`] from
/// a command `Ctx`, so the create-only rule can be exercised after staff have
/// registered the id themselves.
struct Ensure(GameEconomyConfig);

impl GameModule for Ensure {
    fn name(&self) -> &'static str {
        "ensure"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        let cfg = self.0.clone();
        reg.command(
            CommandSpec {
                name: "+ensure".into(),
                shape: ArgShape::Raw,
                help: "+ensure  --  register the settlement currency if it is missing".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                tacsim::economy::ensure_instrument(ctx, &cfg);
                Ok(())
            }),
        )?;
        Ok(())
    }
}

#[test]
fn an_instrument_that_already_exists_is_left_exactly_as_it_is() {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(EconomyModule::new()),
        Box::new(Ensure(GameEconomyConfig::default())),
    ]);
    // Staff registered the id first, at scale 0 and under the economy's own code.
    tw.run(GOD, &format!("@instrument/register {CREDIT}=0"));
    let before = credit_scale(&tw).expect("@instrument/register registered the id");
    assert_eq!(before.1, 0);
    tw.run(GOD, "+ensure");
    let after = credit_scale(&tw).expect("the instrument is still there");
    assert_eq!(
        after, before,
        "create-only: an existing instrument is not touched"
    );
    assert_ne!(after.0, CREDIT_CODE);
}

#[test]
fn ensuring_twice_registers_once() {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(EconomyModule::new()),
        Box::new(Ensure(GameEconomyConfig::default())),
    ]);
    assert!(credit_scale(&tw).is_none());
    tw.run(GOD, "+ensure");
    assert_eq!(
        credit_scale(&tw),
        Some((CREDIT_CODE.to_string(), CREDIT_SCALE))
    );
    tw.run(GOD, "+ensure");
    assert_eq!(
        credit_scale(&tw),
        Some((CREDIT_CODE.to_string(), CREDIT_SCALE))
    );
}

#[test]
fn an_unusable_instrument_id_is_logged_not_fatal() {
    // The host validates `[tacsim.economy].instrument` before composing; the
    // sheet's own boot still never fails on it.
    let tw = TestWorld::with_modules(vec![
        Box::new(EconomyModule::new()),
        Box::new(JobsModule::new()),
        Box::new(PilotRuleset::with_seed(SEED).economy(GameEconomyConfig {
            instrument: "not an id!".into(),
            ..GameEconomyConfig::default()
        })),
    ]);
    assert!(credit_scale(&tw).is_none());
}
