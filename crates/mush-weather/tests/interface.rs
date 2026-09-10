//! S6 (doc 14 §4.6): the meteorologist's interface over a fixture cell — the
//! report and the forecast read the cell the asker stands under, an indoor
//! room is refused, and neither rolls the machine.

mod common;

use mush_core::{
    AbilityCall, Ctx, GOD, InterfaceError, InterfaceReply, ObjId, Outbox, ParsedArgs, SwitchSet,
};
use mush_testkit::TestWorld;
use smol_str::SmolStr;

fn invoke(
    tw: &mut TestWorld,
    asker: ObjId,
    ability: &str,
) -> Result<InterfaceReply, InterfaceError> {
    let engine = tw.engine_mut();
    let provider = engine
        .registry
        .interface_provider("weather")
        .expect("the weather interface");
    let mut out = Outbox::new();
    let mut ctx = Ctx {
        w: &mut engine.world,
        actor: asker,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(SmolStr::new("npc")),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    provider.invoke(
        &mut ctx,
        AbilityCall { npc: GOD, asker },
        ability,
        &serde_json::json!({}),
    )
}

#[test]
fn the_meteorologist_reads_the_cell_the_asker_stands_under() {
    let mut tw = common::world(60, 0);
    let plaza = common::downtown_with_climate(&mut tw);
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, plaza)
        .expect("Rae to the plaza");
    let (state, _) = common::cell_state(&tw, common::DOWNTOWN).expect("a cell");
    // Drain the boot's own writes: the probe is about the reads below.
    let _ = tw.engine_mut().take_module_ops();
    let report = invoke(&mut tw, rae, "weather.report").expect("a report");
    assert!(report.text.contains("°C"), "{}", report.text);
    assert!(report.text.contains("wind"), "{}", report.text);
    let forecast = invoke(&mut tw, rae, "weather.forecast").expect("a forecast");
    assert!(forecast.text.contains("now"), "{}", forecast.text);
    // Neither rolled the machine: the cell is where it was.
    let (after, _) = common::cell_state(&tw, common::DOWNTOWN).expect("a cell");
    assert_eq!(after, state);
    let ops = tw.engine_mut().take_module_ops();
    assert!(ops.is_empty(), "a read persisted nothing: {ops:?}");
    // An unknown ability is unknown.
    assert!(matches!(
        invoke(&mut tw, rae, "weather.summon"),
        Err(InterfaceError::Unknown(_))
    ));
}

#[test]
fn indoors_there_is_no_weather_to_speak_of() {
    let mut tw = common::world(60, 0);
    let _plaza = common::downtown_with_climate(&mut tw);
    let rae = tw.spawn_player("Rae");
    // The spawn room bears no weather flag.
    let err = invoke(&mut tw, rae, "weather.report").expect_err("indoors");
    assert!(matches!(err, InterfaceError::Refused(m) if m.contains("no weather")));
}
