//! The M5 game-clock demo, in two halves per the milestone's guidance:
//!
//! - a **direct, deterministic** harness (`mush-testkit`, no sockets, a
//!   manually-fed clock) for the time-sensitive assertion that IC time
//!   advances at the configured IC:OOC ratio — real sleeps would be flaky;
//! - a light **over-SSH** check that the `@time` command renders the
//!   in-character clock to a connected player, through the whole stack.

use crate::support;

use mush_core::{CivilDate, GOD, GameClock};
use mush_testkit::TestWorld;

use support::{boot_module_server, play_client, scratch_dir};

const GM_PASSWORD: &str = "gm-secret-1";
const BANNER: &str = "Welcome to the clock e2e MUSH!";

/// A clock at `ratio` IC:OOC seconds, calendar-phased so OOC unix 0 reads the
/// given IC `epoch`.
fn clock(ratio: u32, epoch: &str) -> GameClock {
    GameClock {
        ic_ratio: ratio,
        epoch_offset: CivilDate::parse(epoch).unwrap().to_ic_seconds(),
    }
}

/// Run `@time` as God and return the `Game time:` line.
fn time_line(tw: &mut TestWorld) -> String {
    let events = tw.run(GOD, "@time");
    tw.seen_by(&events, GOD)
        .into_iter()
        .find(|l| l.contains("Game time:"))
        .unwrap_or_else(|| panic!("@time produced no line; events: {events:?}"))
}

#[test]
fn ic_time_advances_at_the_configured_ratio() {
    let epoch = "2200-06-15 12:00";
    let mut tw = TestWorld::new();

    // Ratio 1: IC time tracks OOC time one-for-one. One real hour later, the
    // in-character clock has moved one hour.
    tw.world_mut().set_game_clock(clock(1, epoch));
    tw.set_now(0);
    assert!(
        time_line(&mut tw).contains("June 15, 2200, 12:00"),
        "at OOC 0 the clock reads the epoch: {}",
        time_line(&mut tw)
    );
    tw.set_now(3_600);
    let one_to_one = time_line(&mut tw);
    assert!(
        one_to_one.contains("June 15, 2200, 13:00"),
        "ratio 1: +1 OOC hour = +1 IC hour: {one_to_one}"
    );

    // Ratio 24: the same real hour is a full in-character day — IC time
    // demonstrably advances *faster* as the ratio rises.
    tw.world_mut().set_game_clock(clock(24, epoch));
    tw.set_now(3_600);
    let fast = time_line(&mut tw);
    assert!(
        fast.contains("June 16, 2200, 12:00"),
        "ratio 24: +1 OOC hour = +1 IC day: {fast}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn time_command_renders_over_ssh() {
    let dir = scratch_dir("clock-e2e");
    let addr = boot_module_server(&dir, BANNER, GM_PASSWORD).await;

    let (_god_handle, mut god) = play_client(addr).await;
    god.expect(BANNER).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo #0").await;

    let mark = god.mark();
    god.send_line("@time").await;
    god.expect_after(mark, "Game time:").await;
}
