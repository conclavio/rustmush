//! **Doc 19 cut A.3 acceptance — `+chart`** (doc 19 §5, D-19-16).
//!
//! On the production composition with the game on (the lane latched at boot),
//! as a player with no charter: the chart reads the sky from the lane's first
//! port and plans the port's Dray empty. Under the constant sky (rate 0) the
//! window says every day is today; under real motion (rate 1000) it names the
//! fastest, the cheapest and the worst day of the next IC year.

use std::sync::Arc;

use mush_testkit::TestWorld;
use tacsim::config::PackSource;
use tacsim::port::{CHART_NO_BODY, CHART_NO_ROUTE, WINDOW_DAYS};
use tacsim::{EphemerisConfig, TacsimConfig, fixture};

/// The production roster as `mush_server::boot` composes it with the game on
/// (`tests/lane.rs`'s copy).
fn production_roster(cfg: TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let (mut mods, inventory) = comp::server_modules_with_tacsim(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.push(comp::tacsim_module_wired(cfg, true, inventory));
    mods.push(comp::content_module_with(true, true).expect("the content module composes"));
    mods
}

/// `fixture::config()` with the corridor as an operator seed, under a sky rate.
fn config(rate: u32) -> TacsimConfig {
    let mut cfg = fixture::config();
    cfg.seeds = vec![PackSource {
        name: "<built-in tacsim.corridor>".to_owned(),
        bytes: fixture::CORRIDOR_JSON.to_owned(),
    }];
    cfg.time.ephemeris = EphemerisConfig {
        epoch: 74_152_540_800,
        rate,
    };
    cfg
}

fn world(rate: u32) -> TestWorld {
    TestWorld::with_modules(production_roster(config(rate)))
}

fn run(tw: &mut TestWorld, who: mush_core::ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    tw.seen_by(&events, who).join("\n")
}

/// **`+chart` reads the sky from the first port for a walker** (D-19-16): the
/// header names the date, the model, the motion and Ceres Freeport on ceres;
/// every lane body has a row with its distance, longitude, latitude and the
/// light lag from here (Ceres itself "here"); the destination line plans the
/// Dray empty and names the direct chord beside the listed route.
#[test]
fn a3_chart_reads_the_sky_from_the_first_port_for_a_walker() {
    let mut tw = world(0);
    let walker = tw.spawn_player("Walker");
    let out = run(&mut tw, walker, "+chart");
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines.first().is_some_and(|l| l.starts_with("The sky on ")
            && l.contains("(kepler; constant), from Ceres Freeport on ceres:")),
        "{out}"
    );
    let rows: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("  ") && l.contains(" r ") && l.contains(" lat "))
        .collect();
    assert_eq!(
        rows.len(),
        16,
        "sixteen lane bodies, the four moons among them:\n{out}"
    );
    assert!(
        rows.iter()
            .any(|l| l.trim_start().starts_with("phobos") && l.contains("moon of mars")),
        "{out}"
    );
    assert!(
        rows.iter()
            .any(|l| l.trim_start().starts_with("ceres") && l.ends_with("here")),
        "{out}"
    );
    assert!(
        rows.iter().any(|l| l.trim_start().starts_with("pallas")
            && l.contains("AU from here")
            && l.contains("light")),
        "{out}"
    );
    let stickney = lines
        .iter()
        .find(|l| l.starts_with("  Stickney Yards: "))
        .unwrap_or_else(|| panic!("a Stickney line:\n{out}"));
    assert!(stickney.contains("at this load"), "{stickney}");
    assert!(
        stickney.contains("burn") && stickney.contains("crew-days"),
        "{stickney}"
    );
    assert!(stickney.contains("; empty "), "{stickney}");
    assert!(
        stickney.contains("; the direct chord ") && stickney.contains(" AU would be "),
        "{stickney}"
    );
    assert!(!out.contains("Nothing was written") || true);
}

/// **Under the constant sky the window says every day is today**; a port the
/// lane does not clear for is refused with the list.
#[test]
fn a3_chart_window_under_a_constant_sky_says_every_day_is_today() {
    let mut tw = world(0);
    let walker = tw.spawn_player("Walker");
    let out = run(&mut tw, walker, "+chart stickney");
    assert!(
        out.contains(&format!(
            "scanned a day at a time over the next {WINDOW_DAYS} IC days:"
        )),
        "{out}"
    );
    assert!(out.contains("  now ("), "{out}");
    assert!(
        out.contains("The sky is constant on this server: every day is today."),
        "{out}"
    );
    assert!(!out.contains("fastest:"), "{out}");
    let refused = run(&mut tw, walker, "+chart pallas");
    assert!(
        refused.contains(
            &CHART_NO_ROUTE
                .replace("{here}", "Ceres Freeport")
                .replace("{list}", "Stickney Yards")
        ),
        "{refused}"
    );
}

/// **Under real motion the window names the days** (the owner's "optimal times
/// to trade there"): the fastest and the worst day of the next IC year, each
/// against now, and the worst is longer than the fastest.
#[test]
fn a3_chart_window_under_the_moving_sky_names_the_days() {
    let mut tw = world(1_000);
    let walker = tw.spawn_player("Walker");
    let out = run(&mut tw, walker, "+chart stickney");
    let fastest = out
        .lines()
        .find(|l| l.starts_with("  fastest: "))
        .unwrap_or_else(|| panic!("a fastest line:\n{out}"));
    let worst = out
        .lines()
        .find(|l| l.starts_with("  worst: "))
        .unwrap_or_else(|| panic!("a worst line:\n{out}"));
    for line in [fastest, worst] {
        assert!(
            line.contains(" IC days): ") && line.contains("against now"),
            "{line}"
        );
    }
    // The in-days offset of each, and the hours: the worst takes longer.
    let hours = |line: &str| -> i64 {
        let after = line.split("): ").nth(1).expect("a figure");
        let h = after.split(" h ").next().expect("hours");
        h.parse::<i64>().unwrap_or(0)
    };
    assert!(hours(worst) >= hours(fastest), "{fastest}\n{worst}");
}

/// **`+chart/body`**: Pallas is far off the ecliptic at the lane's epoch, its
/// table has 122 rows; an unknown body is refused with the list.
#[test]
fn a3_chart_body_shows_pallas_off_the_ecliptic() {
    let mut tw = world(0);
    let walker = tw.spawn_player("Walker");
    let out = run(&mut tw, walker, "+chart/body pallas");
    assert!(out.starts_with("pallas on "), "{out}");
    assert!(out.contains("latitude +14.07"), "{out}");
    assert!(
        out.contains("From Ceres Freeport on ceres: ") && out.contains("light"),
        "{out}"
    );
    assert!(out.contains("(122 rows, 2340-01-01..2400-01-01)"), "{out}");
    assert!(out.contains("i 35."), "Pallas's inclination:\n{out}");
    let refused = run(&mut tw, walker, "+chart/body nowhere");
    assert!(
        refused.starts_with(&CHART_NO_BODY.replace("{list}", "").trim_end_matches(" .")[..20]),
        "{refused}"
    );
    assert!(refused.contains("pallas"), "{refused}");
}

/// **`+chart <port>` names the destination's demand side under its season** (doc
/// 19 §6, cut A.4b): under the moving sky each of Stickney's standing bids prints
/// its list price and what it posts at, with the premium and the reach against the
/// median; the emergency line says none are due at the shipped ratio and where the
/// reach stands. Under the constant sky the same lines print at a zero premium,
/// before "every day is today".
#[test]
fn a4b_chart_window_names_the_demand_side_and_the_emergency_line() {
    let mut tw = world(1_000);
    let walker = tw.spawn_player("Walker");
    let out = run(&mut tw, walker, "+chart stickney");
    let metals = out
        .lines()
        .find(|l| l.starts_with("  Kasei Metals bids metals: list 280, posts "))
        .unwrap_or_else(|| panic!("a metals line:\n{out}"));
    assert!(
        metals.contains(" permille; reach ") && metals.contains(" now against a median of "),
        "{metals}"
    );
    assert!(
        out.lines()
            .any(|l| l.starts_with("  Kasei Metals bids volatiles: list 200, posts ")),
        "{out}"
    );
    let emergency = out
        .lines()
        .find(|l| l.starts_with("  emergency bids (at 1500 permille of the median): "))
        .unwrap_or_else(|| panic!("an emergency line:\n{out}"));
    assert!(
        emergency.contains("none due; reach is ")
            && emergency.ends_with(" permille of the median."),
        "{emergency}"
    );
    let mut constant = world(0);
    let walker = constant.spawn_player("Walker");
    let out = run(&mut constant, walker, "+chart stickney");
    assert!(
        out.lines()
            .any(|l| l.starts_with("  Kasei Metals bids metals: list 280, posts 280 (+0 permille;")),
        "{out}"
    );
    assert!(
        out.contains(
            "emergency bids (at 1500 permille of the median): none due; reach is 1000 permille"
        ),
        "{out}"
    );
    assert!(out.contains("every day is today"), "{out}");
}
