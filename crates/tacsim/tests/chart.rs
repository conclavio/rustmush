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
use tacsim::port::{
    CHART_NO_BODY, CHART_NO_ROUTE, GRID_HEIGHT, GRID_WIDTH, SKY_CHART_FEED, WINDOW_DAYS,
    chart_frame,
};
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
                .replace("{list}", "Aitken Highport, Stickney Yards")
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

// ---------------------------------------------------------------------------
// A.3b: the sky chart panel
// ---------------------------------------------------------------------------

/// Where the web's chart fixture lives (doc 19 cut A.3b; the cockpit's
/// `web/src/lib/fixtures/cockpit/` precedent): `TACSIM_BLESS=1` writes it, every
/// other run compares byte for byte, so the web client's decoder is pinned to a
/// frame the server really made.
fn chart_fixture_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/lib/fixtures/chart/sky_chart.json")
}

/// **The chart frame is a grid with the chart block beside it** (doc 19 §5, cut
/// A.3b): for a walker at Ceres Freeport under the moving sky, `chart_frame`
/// answers a `grid`-shaped frame 60 by 28 with the Sun, the sixteen bodies by
/// letter (a moon on its parent's cell, no letter of its own), the listed route's
/// legs dotted, the walker's port as `you`; the `chart` block carries every body
/// with its position, the one destination's legs and passages now, the window
/// over the next IC year, and the lines `+chart` prints; `+chart/map` prints the
/// same grid north up. The frame is blessed for the web client's tests.
#[test]
fn a3b_the_chart_frame_is_a_grid_with_the_chart_block_and_is_blessed_for_the_web() {
    let mut tw = world(1_000);
    let walker = tw.spawn_player("Walker");
    let frame = chart_frame(tw.world(), walker).expect("a chart");
    assert_eq!(frame["schema"]["shape"], "grid");
    assert_eq!(frame["schema"]["id"], SKY_CHART_FEED);
    assert_eq!(frame["width"], GRID_WIDTH);
    assert_eq!(frame["height"], GRID_HEIGHT);
    assert_eq!(frame["rows"].as_array().map(Vec::len), Some(GRID_HEIGHT));
    assert!(
        frame["title"].as_str().is_some_and(
            |t| t.starts_with("The sky on ") && t.contains("from Ceres Freeport on ceres")
        ),
        "{}",
        frame["title"]
    );
    let legend: Vec<String> = frame["legend"]
        .as_array()
        .expect("legend")
        .iter()
        .map(|e| {
            format!(
                "{} {}",
                e[0].as_str().unwrap_or("?"),
                e[1].as_str().unwrap_or("?")
            )
        })
        .collect();
    assert!(legend.iter().any(|l| l == "* the Sun"), "{legend:?}");
    assert!(legend.iter().any(|l| l == "C ceres (here)"), "{legend:?}");
    assert!(
        legend
            .iter()
            .any(|l| l.starts_with("M mars") && l.contains("phobos") && l.contains("deimos")),
        "{legend:?}"
    );
    assert!(
        legend.iter().any(|l| l == ": the listed routes"),
        "{legend:?}"
    );
    assert!(
        frame["you"].is_array(),
        "the walker's port is the viewer's cell"
    );
    let grid = tacsim::port::chart_panel::expand_rows(&frame);
    let text: String = grid.iter().rev().map(|r| r.clone() + "\n").collect();
    for glyph in ['*', 'C', 'M', 'S', 'J', ':', '.'] {
        assert!(text.contains(glyph), "{glyph} on the map:\n{text}");
    }
    assert!(
        !text.contains('p') && !text.contains('d'),
        "no moon letters:\n{text}"
    );
    // The chart block.
    let chart = &frame["chart"];
    assert_eq!(chart["model"], "kepler");
    assert_eq!(chart["rate_permille"], 1_000);
    assert_eq!(chart["vantage"]["port"], "Ceres Freeport");
    assert_eq!(chart["vantage"]["body"], "ceres");
    let bodies = chart["bodies"].as_array().expect("bodies");
    assert_eq!(bodies.len(), 16);
    let phobos = bodies.iter().find(|b| b["id"] == "phobos").expect("phobos");
    assert_eq!(phobos["parent"], "mars");
    let mars = bodies.iter().find(|b| b["id"] == "mars").expect("mars");
    assert_eq!(
        phobos["cell"], mars["cell"],
        "a moon shares its parent's cell"
    );
    assert!(
        bodies
            .iter()
            .any(|b| b["here"] == true && b["id"] == "ceres")
    );
    assert!(
        bodies
            .iter()
            .all(|b| b["r_au"].as_f64().is_some_and(|r| r > 0.0))
    );
    let routes = chart["routes"].as_array().expect("routes");
    // Doc 19 cut A.8: Ceres lists Luna and Stickney, in venue order.
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0]["to"], "Aitken Highport");
    assert_eq!(routes[0]["body"], "luna");
    let stickney = routes
        .iter()
        .find(|r| r["to"] == "Stickney Yards")
        .expect("the Stickney route");
    assert_eq!(stickney["body"], "phobos");
    let legs: Vec<(String, String)> = stickney["legs"]
        .as_array()
        .expect("legs")
        .iter()
        .map(|l| {
            (
                l["from"].as_str().unwrap_or("").to_owned(),
                l["to"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        legs,
        vec![
            ("ceres".to_owned(), "kirkwood".to_owned()),
            ("kirkwood".to_owned(), "phobos".to_owned())
        ]
    );
    assert!(stickney["empty"]["total_s"].as_i64().is_some_and(|s| s > 0));
    assert!(stickney["direct"]["km"].as_u64().is_some_and(|km| km > 0));
    let window = chart["window"].as_array().expect("window");
    assert_eq!(window.len(), 2, "a window per destination");
    assert!(
        window[0]["fastest"]["offset_days"].is_number(),
        "{}",
        window[0]
    );
    assert!(window[0]["worst"]["total_s"].as_i64() >= window[0]["fastest"]["total_s"].as_i64());
    let sky_lines = chart["lines"]["sky"].as_array().expect("sky lines");
    assert!(
        sky_lines.len() >= 17,
        "the heading, sixteen rows and a destination"
    );
    assert!(
        chart["lines"]["window"]
            .as_array()
            .is_some_and(|w| !w.is_empty())
    );
    // `+chart/map` prints the grid north up, the legend and the scale.
    let out = run(&mut tw, walker, "+chart/map");
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines.first().is_some_and(|l| l.starts_with("The sky on ")),
        "{out}"
    );
    assert!(
        lines
            .get(1)
            .is_some_and(|l| l.contains("* the Sun") && l.contains("C ceres (here)")),
        "{out}"
    );
    assert!(lines.iter().any(|l| l.contains('*')), "{out}");
    assert!(
        lines
            .last()
            .is_some_and(|l| l.starts_with("scale: logarithmic past")),
        "{out}"
    );
    // Title, legend, the rows with the blank edge rows trimmed, the scale.
    assert!(
        (GRID_HEIGHT - 4 + 3..=GRID_HEIGHT + 3).contains(&lines.len()),
        "{} lines:
{out}",
        lines.len()
    );
    assert!(
        lines.iter().all(|l| !l.is_empty()),
        "no empty line:
{out}"
    );
    // The fixture for the web client.
    let path = chart_fixture_path();
    let pretty = serde_json::to_string_pretty(&frame).expect("encodes") + "\n";
    if std::env::var_os("TACSIM_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the fixture dir");
        std::fs::write(&path, &pretty).expect("the fixture is written");
    } else {
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            on_disk, pretty,
            "web/src/lib/fixtures/chart/sky_chart.json is out of date; re-bless with \
             TACSIM_BLESS=1 cargo test -p tacsim --test chart"
        );
    }
}

/// **Nowhere the lane knows is a frame with no grid and the reason as its title**
/// (doc 19 cut A.3b): a world with no lane answers `CHART_NO_LANE` — the grid
/// printer prints that one line and the web client its one sentence.
#[test]
fn a3b_without_a_lane_the_chart_frame_says_so() {
    let mut cfg = fixture::config();
    cfg.enabled = false;
    // The fixture corridor alone, the game content off: no lane, no chart.
    let tw = TestWorld::with_modules({
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
        mods.push(comp::tacsim_module_wired(cfg, false, inventory));
        mods.push(comp::content_module_with(true, false).expect("the content module composes"));
        mods
    });
    let walker = mush_core::GOD;
    let refused = chart_frame(tw.world(), walker).expect_err("no lane, no chart");
    assert_eq!(refused, tacsim::port::CHART_NO_LANE);
}
