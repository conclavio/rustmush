//! **Doc 41 story (e), whole, with the bot over SSH** (§4.9; cut RS.9b), at 256 (Ahuna Flank) and
//! 512 (Urvara Reach): a raider's wreck left on the site's open ground for 56 IC days is swept by
//! Tycho Mutual's house — titled to its dispatcher as salvage of the unowned, towed home, assessed,
//! restored and offered for sale — and the bot, a player, finds her in `+salvage/forsale`, buys her
//! with `+salvage/buy <hull> go`, and holds her title. The matter census (IN.11's
//! `@industry/census`) holds to the kilogram across the whole of it.
//!
//! **What is a fixture.** God's: the wreck (a prospector hull made with no title, as a raid's hulls
//! are, its chassis destroyed by `@unit/damage`, on the site's index by `@tacsim/site leave`), the
//! bot's credit account and its money, and the world's rate lifted to 3,600 so the 56 IC days pass
//! in real minutes. Every act of the house's is the production path; every one of the player's is
//! typed by the bot.

use std::time::{Duration, Instant};

use mush_bot::mine::lane::salvage;
use mush_core::ObjId;

use crate::fr7_round::{PROSPECTOR, WEST, cid, placed, staff_hand};
use crate::grounds_rig::Ground;
use crate::{arena, mine};

/// The house sweeps after this long, real seconds at the lifted rate, plus its flying: bounded.
const STORY_BOUND: Duration = Duration::from_secs(80 * 60);

/// The chassis destroyed, its internals at the floor, as a mutation plan's JSON: a wreck a
/// workshop can still restore.
fn wrecked_plan() -> String {
    let section = mush_tactics::SectionId::new("chassis").expect("a section");
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![
            mush_tactics::UnitMutationOp::SetCell {
                addr: mush_tactics::CellAddr::Section {
                    section: section.clone(),
                    cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL)
                        .expect("a cell"),
                },
                value: 0,
            },
            mush_tactics::UnitMutationOp::SetSectionCondition {
                section,
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            },
        ],
        None,
        Some("bot-story:rs9-wreck".into()),
    )
    .expect("a plan");
    serde_json::to_string(&plan).expect("the plan's JSON")
}

/// **The fixture**, on the booted engine: the wreck on `ground`'s open ground, four cells east and
/// three north of the stern ramp's cell, and on the site's index. Answers `(the wreck, the bot)`.
fn fixture(
    engine: &mut mush_core::Engine,
    ground: &Ground,
    character: &str,
) -> (Vec<String>, ObjId, ObjId) {
    let (x, y) = tacsim::surface::excursion::site_pad_cm(&engine.world, &cid(ground.entry));
    let side = i64::from(tacsim::surface::ground::SITE_CELL_CM);
    let ramp = (x.div_euclid(side), y.div_euclid(side) - 4);
    let at = (ramp.0 + 4, ramp.1 + 3);
    let wreck = placed(engine, ground, PROSPECTOR, "Raider Wreck", at, WEST);
    let bot = staff_hand(engine, |ctx| {
        ctx.w
            .iter()
            .find(|row| row.name == character)
            .map(|row| row.id)
            .expect("the bot's character")
    });
    let said = vec![
        mine::staff_line(
            engine,
            &format!("@unit/damage #{}={}", wreck.0, wrecked_plan()),
        ),
        mine::staff_line(
            engine,
            &format!(
                "@tacsim/site {} leave #{} wreck {},{}",
                ground.entry, wreck.0, at.0, at.1
            ),
        ),
    ];
    (said, wreck, bot)
}

/// **The bot's money** (a staff seed): God opens its credit account and mints it 5,000,000.00cr
/// from a system source.
async fn fund(god: &mut arena::Client, bot: ObjId) {
    let id = |text: &str, after: &str| {
        text.split(after)
            .nth(1)
            .and_then(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse::<u64>().ok()
            })
            .unwrap_or_else(|| panic!("no account id after {after:?}:\n{text}"))
    };
    let source = god
        .say(
            "@account/system rs9_float=tacsim.credit",
            "Opened account #",
        )
        .await;
    let source = id(&source, "Opened account #");
    let opened = god
        .say(
            &format!("@account/open #{}=tacsim.credit", bot.0),
            "Opened account #",
        )
        .await;
    let account = id(&opened, "Opened account #");
    god.say(
        &format!("@econ/mint {source}={account} 500000000"),
        "Minted",
    )
    .await;
}

/// The census's balance once it holds still, three reads five seconds apart.
async fn steady(god: &mut arena::Client) -> i128 {
    let mut before = crate::in11_matter::census(god).await.0;
    let mut held = 0;
    for _ in 0..60 {
        god.idle(Duration::from_secs(5)).await;
        let now = crate::in11_matter::census(god).await.0;
        if now == before {
            held += 1;
            if held >= 3 {
                return now;
            }
        } else {
            held = 0;
            before = now;
        }
    }
    panic!("the matter census never held still: {before}");
}

/// A salvage line the bot types, in its vocabulary, answered by one of `needles`.
async fn typed(
    client: &mut arena::Client,
    line: &str,
    needles: &[&str],
) -> (Option<usize>, String) {
    assert!(salvage::in_vocabulary(line), "{line}");
    client.answer(line, needles).await
}

/// **Story (e) on `ground`**, the bot `miner`.
pub(crate) async fn story_e(miner: mine::Miner, ground: &'static Ground) {
    let started = Instant::now();
    let site = ground.entry;
    let stamp = |what: &str| println!("rs9e {site}: +{} s {what}", started.elapsed().as_secs());
    let mut made = None;
    let (rig, said) = mine::boot_with(
        &[miner],
        |_| {},
        |engine| {
            let (said, wreck, bot) = fixture(engine, ground, miner.character);
            made = Some((wreck, bot));
            said
        },
    )
    .await;
    let (wreck, bot) = made.expect("the fixture ran");
    stamp(&format!("the wreck #{} on the index: {said:?}", wreck.0));
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut client =
        arena::Client::login(&rig, miner.account, arena::PILOT_PASSWORD, miner.character).await;
    fund(&mut god, bot).await;
    let matter = steady(&mut god).await;
    stamp(&format!("census: balance {matter}"));
    let lifted = mine::lift_rate(&mut god, 3_600).await;
    stamp(&format!(
        "the rate lifted: {}",
        lifted.lines().next().unwrap_or("")
    ));

    // The house sweeps, restores and lists her; the bot reads the list until she is on it.
    let job = format!("#{}", wreck.0);
    let none = tacsim::port::house_sale::NONE_FOR_SALE;
    let mut polls = 0u32;
    let listed = loop {
        let (index, text) = typed(&mut client, "+salvage/forsale", &[job.as_str(), none]).await;
        if index == Some(0) {
            break text;
        }
        polls += 1;
        if polls % 6 == 1 {
            let (_, time) = god.answer("@tacsim/time", &["IC", "rate"]).await;
            let (_, index) = god
                .answer(
                    &format!("@tacsim/site {site}"),
                    &["wreck", "casualty", "units"],
                )
                .await;
            let (_, quote) = god
                .answer(
                    &format!("+bay/quote {job}"),
                    &["Condition", "no bay", "scrapped", "No such"],
                )
                .await;
            stamp(&format!(
                "poll {polls}: the wreck: {:?}",
                quote
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .take(3)
                    .collect::<Vec<_>>()
            ));
            stamp(&format!(
                "poll {polls}: time {:?}; the site {:?}",
                time.lines()
                    .find(|l| l.contains("IC") || l.contains("rate"))
                    .unwrap_or(""),
                index
                    .lines()
                    .filter(|l| l.contains(&job))
                    .collect::<Vec<_>>()
            ));
        }
        assert!(
            started.elapsed() < STORY_BOUND,
            "the house never listed the wreck in {} s:\n{text}",
            started.elapsed().as_secs()
        );
        client.idle(Duration::from_secs(20)).await;
    };
    stamp(&format!(
        "for sale: {}",
        listed.lines().find(|l| l.contains(&job)).unwrap_or("")
    ));

    // The bot buys her: the preview, then go.
    let buy = format!("+salvage/buy {job}");
    let (index, text) = typed(&mut client, &buy, &["Add go"]).await;
    assert_eq!(index, Some(0), "the preview:\n{text}");
    let go = format!("{buy} go");
    let (index, text) = typed(&mut client, &go, &["Registry writes the sale"]).await;
    assert_eq!(index, Some(0), "the sale:\n{text}");
    stamp("the sale filed");

    // Her title passes at the next settlement: the bay reads the bot's right as her title.
    let quote = format!("+bay/quote {job}");
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    loop {
        let (index, text) = client.answer(&quote, &["Your right: title"]).await;
        if index == Some(0) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "her title never passed to the bot:\n{text}"
        );
        client.idle(Duration::from_secs(10)).await;
    }
    stamp("her title is the bot's");

    // The census holds across the sweep, the restoration and the sale.
    let after = crate::in11_matter::census(&mut god).await.0;
    assert_eq!(after, matter, "the matter census holds across story (e)");
    stamp(&format!("census: balance {after}, held"));
    client.close().await;
    god.close().await;
    drop(rig);
}
