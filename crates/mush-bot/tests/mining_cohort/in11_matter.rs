//! **Doc 39 cut IN.11 — the matter story, the bot over SSH at 256 and 512** (§6's IN.11 row;
//! Q-39-8, Q-38-29). The mining lane plays its loop — kit-up, the hop, the mine, home, the hold
//! sold into the Exchange's ore bid (the Exchange mills it) — and then the bot types the
//! production half, every line in its closed vocabulary and every answer read by its own reader
//! ([`mush_bot::mine::lane::works`]):
//!
//! 1. At Ceres: concentrate at the Exchange's ask, two tonnes of carbon ore and two of strategic
//!    ore at Freeport Works' (lane v30), and by Luna (a tonne of instrument boards at Crisium's
//!    ask) and Deimos to Stickney Yards (the corridor's outbound leg is interdicted at Kirkwood,
//!    whose picket's words the bot does not have; IN.5's way).
//! 2. At Stickney: the concentrate into Kasei's bid, refined into alloys; ten tonnes of alloys
//!    and one of fittings bought.
//! 3. The works: fittings, five light plants, ten sights.
//! 4. The yard: a prospector from them (`+yard/order tv_prospector`), commissioned on the floor.
//! 5. A light plant sold into the Yards' bid (`+market/sell <bid>=1`).
//! 6. The prospector's plant destroyed by God (a fight's stand-in), the replace quote read
//!    (`+bay/quote #<n>`, a replace among its jobs), the prospector stowed in the bot's Dray-P, berthed from it,
//!    and a light plant fitted from the hold (`+bay/fit #<n>=power <good>`).
//!
//! God types `@industry/census` when the bot logs in and at the end: the `tacsim.matter_census`
//! line's balance does not move across the whole story, mining included, and every
//! `tacsim.matter_build` line reads inputs = the hull + the waste.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the float (God mints the pilot 50,000.00cr), and the plant's
//! destruction. **Tier** (TS.2): dispatch / by-hand, `#[ignore]`d, run by name (`in11_`).

use std::time::Duration;

use mush_bot::mine::lines;

use super::{Miner, arena, bunker, buy, float, mine, sail, sell_all, typed};

/// The production half's world rate: an IC hour a real second.
const RATE: u32 = 3_600;
const PLANT: &str = "tacsim.part.plant.light_mk1";
const SIGHT: &str = "tacsim.part.sensor.visual_mk1";

/// The census line's terms, typed by God.
pub(crate) async fn census(god: &mut arena::Client) -> (i128, Vec<(i128, i128, i128)>) {
    let from = god.seen.len();
    let (index, text) = god
        .answer("@industry/census", &["tacsim.matter_census"])
        .await;
    assert!(index.is_some(), "the census answered:\n{text}");
    // The build lines follow the census line: read the whole answer.
    god.idle(Duration::from_secs(2)).await;
    let text = lines::normalise(god.seen.get(from..).unwrap_or(""));
    let terms = |line: &str| -> std::collections::BTreeMap<String, i128> {
        line.split_whitespace()
            .filter_map(|w| w.split_once('='))
            .filter_map(|(k, v)| v.parse().ok().map(|v| (k.to_owned(), v)))
            .collect()
    };
    let balance = text
        .split("tacsim.matter_census")
        .nth(1)
        .map(|rest| terms(rest.split("tacsim.matter_build").next().unwrap_or(rest)))
        .and_then(|t| t.get("balance").copied())
        .expect("the census's balance");
    let builds = text
        .split("tacsim.matter_build")
        .skip(1)
        .map(|rest| {
            let t = terms(rest.split("tacsim.matter_census").next().unwrap_or(rest));
            (t["inputs"], t["hull"], t["waste"])
        })
        .collect();
    (balance, builds)
}

/// One order at `verb` (`+works` or `+yard`): booked, its bill delivered line by line, waited
/// for at `front`'s word (`works` or `yard`) and collected; the collection's text.
async fn work(
    client: &mut arena::Client,
    verb: &str,
    front: &str,
    recipe: &str,
    bill: &[(&str, u64)],
) -> (u64, String) {
    use mush_bot::mine::lane::works;
    let booked_at = format!(" booked at the {front}: ");
    let (_, booking) = typed(
        client,
        &format!("{verb}/order {recipe}"),
        &[&booked_at],
        &[],
    )
    .await;
    let order = if front == "yard" {
        works::yard_booked(&booking)
    } else {
        works::booked(&booking)
    }
    .expect("the order's number");
    let from = client.seen.len();
    for (good, units) in bill {
        let (_, text) = typed(
            client,
            &format!("{verb}/deliver {order}={good} {units}"),
            &[works::INTO_ESCROW],
            &["is still settling.", "That hold carries "],
        )
        .await;
        assert!(works::supplied(&text, order), "{text}");
    }
    let delivered = format!("Order {order} is delivered at the {front}: ");
    assert!(
        client
            .watch(from, &delivered, Duration::from_secs(10 * 60))
            .await,
        "order {order} is delivered at the {front}"
    );
    let (_, text) = typed(
        client,
        &format!("{verb}/collect {order}"),
        &[works::INTO_HOLD, works::COMMISSIONS_AS],
        &[works::SETTLING],
    )
    .await;
    (order, text)
}

/// Walk `words` in order, each answered by its room's title.
async fn walk(client: &mut arena::Client, words: &[(&str, &str)]) {
    for (word, room) in words {
        typed(client, word, &[room], &[]).await;
    }
}

/// **The story** at `miner`'s site.
#[allow(clippy::too_many_lines)] // one story, in order
pub(super) async fn matter_story(miner: Miner) {
    use mush_bot::mine::lane::works;
    use mush_bot::mine::{Lane, Milestone, Settings};
    let started = std::time::Instant::now();
    let rig = mine::boot(&[miner]).await;
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut client =
        arena::Client::login(&rig, miner.account, arena::PILOT_PASSWORD, miner.character).await;
    let hull =
        super::hull_name(&format!("{}+{}", miner.account, miner.character)).expect("the hull name");
    let site = miner.site.word();
    let stamp = |what: &str| println!("in11 {site}: +{} s {what}", started.elapsed().as_secs());

    // ---- The window opens, once the houses' boot builds have settled (Tycho's house, doc 41
    // RS.6a, commissions on its first fires): the census's balance holds still three reads ----
    let mut before = census(&mut god).await.0;
    let mut steady = 0;
    for _ in 0..60 {
        god.idle(Duration::from_secs(5)).await;
        let now = census(&mut god).await.0;
        if now == before {
            steady += 1;
            if steady >= 3 {
                break;
            }
        } else {
            steady = 0;
            before = now;
        }
    }
    stamp(&format!("census: balance {before}"));

    // ---- The mining loop: the lane's own, to the hold sold at the Exchange ----
    let mut lane = Lane::new(
        miner.site,
        Settings {
            hull: hull.clone(),
            epoch_unix_ms: mine::unix_now_ms(),
        },
        miner.character.to_owned(),
    );
    let lift = mine::rate_lift(mine::LIFTED_RATE);
    let drive_started = std::time::Instant::now();
    let bound = Duration::from_secs(40 * 60);
    let enrolled_now = |lane: &Lane| lane.step_line() == Some("+coop/refit prospector");
    mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| enrolled_now(lane) || lane.reached(Milestone::Launched),
        bound,
    )
    .await;
    if let (Some(rate), true) = (lift, enrolled_now(&lane)) {
        mine::lift_rate(&mut god, rate).await;
    }
    let (wall, _) = mine::drive_lane_since(
        &mut client,
        &mut lane,
        drive_started,
        |lane| lane.reached(Milestone::Sold),
        bound,
    )
    .await;
    assert!(
        lane.reached(Milestone::Sold),
        "the hold was not sold in {wall} s"
    );
    stamp("the mining loop sold its hold at the Exchange");

    // ---- Ceres: concentrate, carbon and strategic ore ----
    if lift != Some(RATE) {
        mine::lift_rate(&mut god, RATE).await;
    }
    float(&mut god, &mut client).await;
    let rows = super::board_rows(&mut client).await;
    let ask = |label: &str| {
        works::ask_row(&rows, label)
            .map(|row| row.number)
            .unwrap_or_else(|| panic!("the {label} ask"))
    };
    let (concentrate, carbon, strategic) = (
        ask("metal concentrate"),
        ask("carbon ore"),
        ask("strategic ore"),
    );
    buy(&mut client, concentrate, 20).await;
    buy(&mut client, carbon, 2).await;
    buy(&mut client, strategic, 2).await;
    bunker(&mut client, "luna").await;
    walk(
        &mut client,
        &[
            ("concourse", lines::ROOM_CONCOURSE),
            ("arrivals", lines::ROOM_ARRIVALS),
            ("lobby", lines::ROOM_LOBBY),
            ("gallery", lines::ROOM_GALLERY),
        ],
    )
    .await;
    stamp("bought concentrate, carbon and strategic ore; sailing by Luna and Deimos");
    sail(&mut client, &hull, "luna").await;
    let rows = super::board_rows(&mut client).await;
    let boards = works::ask_row(&rows, "instrument boards")
        .map(|row| row.number)
        .expect("Crisium's boards ask");
    buy(&mut client, boards, 1).await;
    stamp("bought a tonne of instrument boards at Aitken Highport");
    bunker(&mut client, "deimos").await;
    sail(&mut client, &hull, "deimos").await;
    bunker(&mut client, "stickney").await;
    sail(&mut client, &hull, "stickney").await;
    stamp("docked at Stickney Yards");

    // ---- Stickney: the concentrate into Kasei's bid, refined; alloys and fittings bought ----
    let rows = super::board_rows(&mut client).await;
    let alloys_before = works::ask_row(&rows, "structural alloy").and_then(works::row_quantity);
    let kasei = works::bid_row(&rows, "metal concentrate").expect("Kasei's concentrate bid");
    let sale = sell_all(&mut client, kasei.number).await;
    assert_eq!(sale.qty_t, 20, "{sale:?}");
    let mut alloys = None;
    for _ in 0..60 {
        let rows = super::board_rows(&mut client).await;
        let ask = works::ask_row(&rows, "structural alloy");
        if let (Some(now), Some(before)) = (ask.and_then(works::row_quantity), alloys_before)
            && now >= before + 16
        {
            alloys = ask.map(|row| row.number);
            break;
        }
        client.idle(Duration::from_secs(5)).await;
    }
    let alloys = alloys.expect("Kasei's alloy ask restocks by the refine's sixteen tonnes");
    buy(&mut client, alloys, 10).await;
    let rows = super::board_rows(&mut client).await;
    let fittings = works::ask_row(&rows, "yard fittings")
        .map(|row| row.number)
        .expect("the Yards' fittings ask");
    buy(&mut client, fittings, 1).await;
    stamp("sold the concentrate to Kasei; bought ten tonnes of alloys and one of fittings");

    // ---- The works: fittings, light plants, sights ----
    let _ = work(
        &mut client,
        "+works",
        "works",
        "fittings",
        &[("structural_alloys", 5), ("carbon_ore", 1)],
    )
    .await;
    let _ = work(
        &mut client,
        "+works",
        "works",
        "plant_light",
        &[("industrial_parts", 1), ("carbon_ore", 1)],
    )
    .await;
    let _ = work(
        &mut client,
        "+works",
        "works",
        "sight_visual",
        &[
            ("strategic_ore", 2),
            ("instrument_boards", 1),
            ("industrial_parts", 3),
        ],
    )
    .await;
    stamp("the works made fittings, five light plants and ten sights");

    // ---- The yard: a prospector ----
    let (order, text) = work(
        &mut client,
        "+yard",
        "yard",
        "tv_prospector",
        &[
            ("structural_alloys", 5),
            ("industrial_parts", 2),
            (PLANT, 1),
            (SIGHT, 1),
        ],
    )
    .await;
    let mark = works::commissioned(&text, order).expect("the commission's mark");
    stamp(&format!("the yard commissions the prospector as {mark}"));
    walk(
        &mut client,
        &[
            ("lobby", "Pressure Lobby"),
            ("customs", "Republic Customs Hall"),
            ("concourse", "Yard Concourse"),
            ("service", "Stickney Service Counter"),
        ],
    )
    .await;
    let (_, look) = typed(
        &mut client,
        &format!("+bay {mark}"),
        &[" stands in a workshop."],
        &["There is no such hull here."],
    )
    .await;
    let built = works::bay_ref(&look, &mark).expect("the prospector's dbref");

    // ---- A light plant sold into the Yards' bid ----
    let rows = super::board_rows(&mut client).await;
    let bid = works::bid_row(&rows, "light power plant").expect("the Yards' light-plant bid");
    // The bid's take as the market reads it (the preview), and the census's lots (diagnostics).
    let (_, preview) = typed(
        &mut client,
        &format!("+market/sell {}", bid.number),
        &["sell", "Sell", "buying"],
        &[],
    )
    .await;
    stamp(&format!(
        "the sale's preview: {}",
        lines::normalise(&preview)
    ));
    let (_, lines_now) = god
        .answer(
            &format!("@industry/census {PLANT}"),
            &["tacsim.matter_census"],
        )
        .await;
    god.idle(Duration::from_secs(2)).await;
    stamp(&format!(
        "census before the sale: {}",
        lines::normalise(&lines_now)
    ));
    let (_, sold) = typed(
        &mut client,
        &format!("+market/sell {}=1", bid.number),
        &[" when the goods are in"],
        &[lines::GOODS_WAIT, lines::SELL_BUSY],
    )
    .await;
    stamp(&format!(
        "sold a light plant to the Yards: {}",
        lines::sale_receipt(&sold).map_or_else(String::new, |s| s.net)
    ));

    // ---- The fit: God destroys her plant; the bot quotes, stows, berths and fits ----
    let plan = super::fr7_round::powerless_plan();
    let (_, damaged) = god
        .answer(&format!("@unit/damage #{built}={plan}"), &["#"])
        .await;
    stamp(&format!("God destroyed {mark}'s plant: {}", damaged.trim()));
    let (_, quote) = typed(
        &mut client,
        &format!("+bay/quote #{built}"),
        &["power", "cr"],
        &[],
    )
    .await;
    stamp(&format!("the replace quote: {}", lines::normalise(&quote)));
    typed(
        &mut client,
        &format!("+bay/stow #{built}={hull}"),
        &["is stowed in the"],
        &[],
    )
    .await;
    typed(
        &mut client,
        &format!("+bay/berth #{built}"),
        &["berthed in the workshop"],
        &[],
    )
    .await;
    typed(
        &mut client,
        &format!("+bay/fit #{built}=power {PLANT}"),
        &["taken in the workshop"],
        &[],
    )
    .await;
    let mut fitted = false;
    for _ in 0..120 {
        let (_, look) = typed(&mut client, &format!("+bay #{built}"), &["Condition:"], &[]).await;
        if !lines::normalise(&look).contains("Condition: disabled.") {
            fitted = true;
            break;
        }
        client.idle(Duration::from_secs(5)).await;
    }
    assert!(fitted, "the light plant is fitted and {mark} runs again");
    stamp(&format!("a light plant fitted into {mark}"));

    // ---- The window closes ----
    let (after, builds) = census(&mut god).await;
    stamp(&format!("census: balance {after}; builds {builds:?}"));
    assert_eq!(before, after, "matter is neither made nor lost");
    assert!(!builds.is_empty(), "the prospector's build line");
    assert!(
        builds
            .iter()
            .all(|(inputs, hull, waste)| *inputs == hull + waste),
        "inputs = the hull + the waste: {builds:?}"
    );
    client.close().await;
    god.close().await;
    drop(rig);
}
