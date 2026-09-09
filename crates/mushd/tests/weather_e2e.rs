//! The M5 "rain rolls in" demo as one end-to-end journey over real SSH
//! (in-memory store, ungated): the full server wiring with the standard M5
//! modules (`mush-weather` among them) exactly as `main.rs` boots them, then
//! a builder assigning a climate to a zone and pinning weather while two
//! players — one in an OUTDOORS room, one INDOORS in the same zone — read
//! (or, for the indoor room, do not read) the results over the wire.
//!
//! Proves, over the wire:
//!
//! - `@weather/climate <zone>=<name>` opts a zone in;
//! - `@weather <zone>=<state>` broadcasts a deterministic transition line to
//!   a player standing in an OUTDOORS room of the zone (the `rain->storm`
//!   pair line);
//! - `look` in that room appends the current-conditions line;
//! - an INDOORS room in the same zone stays silent — its occupant's `look`
//!   shows the room but no weather.

mod support;

use support::{boot_module_server, login_create, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const BANNER: &str = "Welcome to the weather e2e MUSH!";
const CLIMATE: &str = "temperate-coastal";

#[tokio::test(flavor = "multi_thread")]
async fn rain_rolls_in_demo() {
    let dir = scratch_dir("weather-e2e");
    let addr = boot_module_server(&dir, BANNER, GM_PASSWORD).await;

    // --- God (the builder). ---------------------------------------------
    let (_god_handle, mut god) = play_client(addr).await;
    god.expect(BANNER).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo #0").await;

    // --- S2b: weather keys on the atlas. The starter lot is applied at first
    // boot; the OOC Area's rooms are outdoors, and the Staff lot is another
    // neighborhood with no cell of its own — silent.
    god.send_line(&format!("@weather/climate OOC Area={CLIMATE}"))
        .await;
    god.expect("now uses climate").await;

    // --- Two players take their positions. ------------------------------
    // Alice walks out to the OOC bus stop (outdoors, in the OOC Area).
    let (_alice_handle, mut alice) = login_create(addr, "Alice", "secret123").await;
    god.expect("Alice has connected.").await;
    let mark = alice.mark();
    alice.send_line("o").await;
    alice.expect_after(mark, "Welcome Hall #").await;
    let mark = alice.mark();
    alice.send_line("s").await;
    alice.expect_after(mark, "Lounge #").await;
    let mark = alice.mark();
    alice.send_line("out").await;
    alice.expect_after(mark, "Bus Stop (OOC) #").await;

    // Bob goes to the Staff lot's forecourt: a different neighborhood, no cell.
    let (_bob_handle, mut bob) = login_create(addr, "Bob", "hunter42").await;
    let mark = bob.mark();
    bob.send_line("s").await;
    bob.expect_after(mark, "Forecourt & Bus Stop #").await;

    // --- Rain rolls in. -------------------------------------------------
    // Pin to rain, then to storm: the `rain->storm` transition pool has a
    // single line, so the broadcast to the OUTDOORS room is deterministic.
    let mark = god.mark();
    god.send_line("@weather OOC Area=rain").await;
    god.expect_after(mark, "Pinned").await;
    let mark = alice.mark();
    let god_mark = god.mark();
    god.send_line("@weather OOC Area=storm").await;
    god.expect_after(god_mark, "Pinned").await;
    alice
        .expect_after(mark, "The rain hardens and thunder rolls in close.")
        .await;

    // Alice's look shows the current-conditions line (the LookRendered hook).
    let mark = alice.mark();
    alice.send_line("look").await;
    alice.expect_after(mark, "Bus Stop (OOC) #").await;
    alice.expect_after(mark, "Overhead: a storm").await;

    // Bob's INDOORS room stays silent: his look shows the room but no
    // weather. A sentinel `think` after the look guarantees all of the look
    // output has arrived before we assert its absence.
    let mark = bob.mark();
    bob.send_line("look").await;
    bob.expect_after(mark, "Forecourt & Bus Stop #").await;
    bob.send_line("think WEATHER-SENTINEL").await;
    bob.expect_after(mark, "WEATHER-SENTINEL").await;
    let slice = strip_ansi(&bob.seen[mark..]);
    assert!(
        !slice.contains("Overhead:") && !slice.contains("thunder"),
        "indoor room must stay silent; Bob saw:\n{slice}"
    );
}
