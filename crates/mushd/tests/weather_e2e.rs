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

use support::{Term, boot_module_server, login_create, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const BANNER: &str = "Welcome to the weather e2e MUSH!";
const CLIMATE: &str = "temperate-coastal";

/// The K2 busy signal: the world refuses every ordinary line while another
/// action's write is still being acknowledged. Under a loaded full-suite run
/// the first-boot pack apply or a weather pulse can be mid-commit exactly
/// when a player's line lands, so the journey resends the line instead of
/// treating the bounce as a wrong answer — which is what the signal asks for.
const WORLD_BUSY: &str = "The world is committing another action; please retry in a moment.";

/// Send `line` and wait for `needle`, resending after a short pause each time
/// the world answers with the busy signal instead. Returns the mark taken
/// before the send that finally landed, so a caller can read what followed.
async fn send_until(term: &mut Term, line: &str, needle: &str) -> usize {
    for _ in 0..64 {
        let mark = term.mark();
        term.send_line(line).await;
        if term.expect_one_of_after(mark, &[needle, WORLD_BUSY]).await == 0 {
            return mark;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!(
        "{line:?} kept bouncing on the busy signal; seen: {:?}",
        term.seen
    );
}

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
    send_until(
        &mut god,
        &format!("@weather/climate OOC Area={CLIMATE}"),
        "now uses climate",
    )
    .await;

    // --- Two players take their positions. ------------------------------
    // Alice walks out to the OOC bus stop (outdoors, in the OOC Area).
    let (_alice_handle, mut alice) = login_create(addr, "Alice", "secret123").await;
    god.expect("Alice has connected.").await;
    send_until(&mut alice, "o", "Welcome Hall #").await;
    send_until(&mut alice, "s", "Lounge #").await;
    send_until(&mut alice, "out", "Bus Stop (OOC) #").await;

    // Bob goes to the Staff lot's forecourt: a different neighborhood, no cell.
    let (_bob_handle, mut bob) = login_create(addr, "Bob", "hunter42").await;
    send_until(&mut bob, "s", "Forecourt & Bus Stop #").await;

    // --- Rain rolls in. -------------------------------------------------
    // Pin to rain, then to storm: the `rain->storm` transition pool has a
    // single line, so the broadcast to the OUTDOORS room is deterministic.
    send_until(&mut god, "@weather OOC Area=rain", "Pinned").await;
    let mark = alice.mark();
    send_until(&mut god, "@weather OOC Area=storm", "Pinned").await;
    alice
        .expect_after(mark, "The rain hardens and thunder rolls in close.")
        .await;

    // Alice's look shows the current-conditions line (the LookRendered hook).
    let mark = send_until(&mut alice, "look", "Bus Stop (OOC) #").await;
    alice.expect_after(mark, "Overhead: a storm").await;

    // Bob's INDOORS room stays silent: his look shows the room but no
    // weather. A sentinel `think` after the look guarantees all of the look
    // output has arrived before we assert its absence.
    let mark = send_until(&mut bob, "look", "Forecourt & Bus Stop #").await;
    send_until(&mut bob, "think WEATHER-SENTINEL", "WEATHER-SENTINEL").await;
    let slice = strip_ansi(&bob.seen[mark..]);
    assert!(
        !slice.contains("Overhead:") && !slice.contains("thunder"),
        "indoor room must stay silent; Bob saw:\n{slice}"
    );
}
