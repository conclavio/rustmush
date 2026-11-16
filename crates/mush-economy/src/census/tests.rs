#![cfg(test)]
//! The money census (doc 48 cut EX.0): the ledger's arithmetic on hand-built readings, the
//! categories, the 120-column view, the document, and `@econ/census` through the real sweep.

use std::collections::BTreeMap;

use mush_core::{GOD, ObjId};
use mush_org::{OrgId, PrincipalId, SystemPrincipalId};
use mush_testkit::TestWorld;
use mush_testkit::fixtures::{err, ok};

use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn reading(system: &[(u64, i64)], player: i64, lane: i64) -> Reading {
    Reading {
        system: system.iter().map(|(id, m)| (AccountId(*id), *m)).collect(),
        system_rest: 0,
        holdings: Holdings {
            player,
            lane,
            ..Holdings::default()
        },
        partial: false,
    }
}

fn no_labels(account: AccountId) -> String {
    format!("#{}", account.0)
}

#[test]
fn ex0_a_reading_opens_and_the_next_day_closes_a_row() -> R {
    let mut ledger = Ledger::default();
    check!(ledger.due(10));
    // Account 3 holds -40 throughout: an account that did not move is no flow.
    check!(ledger.advance(reading(&[(1, -1_000), (3, -40)], 1_040, 0), 10, 864_000, 5));
    check!(ledger.rows.is_empty());
    check!(!ledger.due(10));
    // The faucet mints 500 more, the sink takes 200 of it: net 300 into the holdings.
    check!(ledger.advance(
        reading(&[(1, -1_500), (2, 200), (3, -40)], 1_340, 0),
        11,
        950_400,
        9
    ));
    let row = ledger.rows.front().ok_or("a row")?;
    check_eq!(row.day, 10);
    check_eq!(row.days, 1);
    check_eq!(row.opened_ic, 864_000);
    check_eq!(row.closed_ic, 950_400);
    check_eq!(
        row.flows,
        vec![
            Flow {
                account: AccountId(1),
                delta: -500
            },
            Flow {
                account: AccountId(2),
                delta: 200
            },
        ]
    );
    check_eq!(row.mint(), 500);
    check_eq!(row.sink(), 200);
    check_eq!(row.net(), 300);
    check_eq!(row.drift(), 0);
    check_eq!(ledger.open.as_ref().map(|o| o.day), Some(11));
    Ok(())
}

#[test]
fn ex0_the_same_day_closes_nothing() -> R {
    let mut ledger = Ledger::default();
    ledger.advance(reading(&[(1, -10)], 10, 0), 3, 0, 0);
    check!(!ledger.advance(reading(&[(1, -99)], 99, 0), 3, 10, 1));
    check!(ledger.rows.is_empty());
    // The open reading is the day's first, untouched.
    check_eq!(
        ledger.open.as_ref().map(|o| o.reading.holdings.player),
        Some(10)
    );
    Ok(())
}

#[test]
fn ex0_a_day_the_sweep_missed_is_one_row_of_two_days() -> R {
    let mut ledger = Ledger::default();
    ledger.advance(reading(&[(1, 0)], 0, 0), 20, 0, 0);
    ledger.advance(reading(&[(1, -40)], 40, 0), 22, 0, 0);
    let row = ledger.rows.front().ok_or("a row")?;
    check_eq!((row.day, row.days, row.net()), (20, 2, 40));
    Ok(())
}

#[test]
fn ex0_the_clock_moving_back_reopens_and_closes_nothing() -> R {
    let mut ledger = Ledger::default();
    ledger.advance(reading(&[(1, -5)], 5, 0), 30, 0, 0);
    check!(ledger.advance(reading(&[(1, -9)], 9, 0), 29, 0, 0));
    check!(ledger.rows.is_empty());
    check_eq!(ledger.open.as_ref().map(|o| o.day), Some(29));
    Ok(())
}

#[test]
fn ex0_rows_are_bounded_oldest_first_out() -> R {
    let mut ledger = Ledger::default();
    let extra = 5;
    let last = i64::try_from(MAX_ROWS + extra).map_err(|e| e.to_string())?;
    for day in 0..=last {
        ledger.advance(reading(&[(1, -day)], day, 0), day, 0, 0);
    }
    check_eq!(ledger.rows.len(), MAX_ROWS);
    check_eq!(
        ledger.rows.front().map(|r| r.day),
        Some(i64::try_from(extra).map_err(|e| e.to_string())?)
    );
    check_eq!(ledger.rows.back().map(|r| r.day), Some(last - 1));
    Ok(())
}

#[test]
fn ex0_a_partial_scan_shows_as_drift() -> R {
    let open = Open {
        day: 1,
        at_ic: 0,
        at_real: 0,
        reading: reading(&[(1, -100)], 100, 0),
    };
    let mut later = reading(&[(1, -150)], 120, 0);
    later.partial = true;
    let row = close(&open, &later, 2, 0, 0);
    check!(row.partial);
    check_eq!(row.net(), 50);
    check_eq!(row.drift(), 30);
    Ok(())
}

#[test]
fn ex0_the_system_accounts_past_the_bound_are_summed() -> R {
    let mut open = reading(&[(1, -10)], 10, 0);
    open.system_rest = -5;
    let mut later = reading(&[(1, -10)], 25, 0);
    later.system_rest = -20;
    let row = close(
        &Open {
            day: 0,
            at_ic: 0,
            at_real: 0,
            reading: open,
        },
        &later,
        1,
        0,
        0,
    );
    check_eq!((row.flow_rest, row.mint(), row.drift()), (-15, 15, 0));
    Ok(())
}

#[test]
fn ex0_an_owner_has_one_category_and_a_system_account_none() -> R {
    let lane = OrgId(1);
    let house = OrgId(2);
    let other_org = OrgId(3);
    let orgs = BTreeMap::from([(lane, HoldingClass::Lane), (house, HoldingClass::House)]);
    let player = ObjId(7);
    let is_player = |id: ObjId| id == player;
    let system = SystemPrincipalId::new("tacsim.treasury").ok_or("system id")?;
    check_eq!(
        classify(&PrincipalId::System(system), &orgs, is_player),
        None
    );
    check_eq!(
        classify(&PrincipalId::Organization(lane), &orgs, is_player),
        Some(HoldingClass::Lane)
    );
    check_eq!(
        classify(&PrincipalId::Organization(house), &orgs, is_player),
        Some(HoldingClass::House)
    );
    check_eq!(
        classify(&PrincipalId::Organization(other_org), &orgs, is_player),
        Some(HoldingClass::Org)
    );
    check_eq!(
        classify(&PrincipalId::Object(player), &orgs, is_player),
        Some(HoldingClass::Player)
    );
    check_eq!(
        classify(&PrincipalId::Object(ObjId(8)), &orgs, is_player),
        Some(HoldingClass::Other)
    );
    Ok(())
}

#[test]
fn ex0_every_line_fits_120_columns_and_a_side_is_capped() -> R {
    let big = 1_000_000_000_000_000i64;
    let open = Open {
        day: 140_000,
        at_ic: 0,
        at_real: 0,
        reading: Reading {
            system: (1..=20u64).map(|id| (AccountId(id), 0)).collect(),
            system_rest: 0,
            holdings: Holdings {
                lane: big,
                house: big,
                org: big,
                player: big,
                other: big,
            },
            partial: false,
        },
    };
    let later = Reading {
        // Ten faucets mint, ten sinks take in, each a large amount.
        system: (1..=20u64)
            .map(|id| (AccountId(id), if id <= 10 { -big } else { big / 2 }))
            .collect(),
        system_rest: 0,
        holdings: Holdings {
            lane: -big,
            house: -big,
            org: -big,
            player: -big,
            other: -big,
        },
        partial: true,
    };
    let row = close(&open, &later, 140_001, 0, 0);
    let long = |account: AccountId| format!("#{} sys:{}", account.0, "x".repeat(80));
    let lines = row_lines(&row, true, &long);
    for line in &lines {
        check!(
            line.chars().count() <= MAX_COLUMNS,
            "{} columns: {line}",
            line.len()
        );
    }
    let sources = lines.iter().filter(|l| l.starts_with("  source #")).count();
    check_eq!(sources, MAX_ACCOUNT_LINES);
    check!(
        lines
            .iter()
            .any(|l| l.starts_with("  source +2 more mint="))
    );
    check!(
        lines
            .iter()
            .any(|l| l.starts_with("  sink +2 more intake="))
    );
    // The day line is carried over two lines at these sizes, and keeps its flags.
    let head = lines.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
    check!(head.ends_with(" partial open"), "{head}");
    Ok(())
}

#[test]
fn ex0_the_window_sums_each_faucets_net_mint() -> R {
    let mut ledger = Ledger::default();
    ledger.advance(reading(&[(1, 0), (2, 0), (3, 0)], 0, 0), 5, 0, 0);
    ledger.advance(reading(&[(1, -100), (2, 0), (3, 10)], 90, 0), 6, 0, 0);
    ledger.advance(reading(&[(1, -300), (2, -50), (3, 30)], 320, 0), 7, 0, 0);
    let rows: Vec<&Row> = ledger.rows.iter().collect();
    let lines = window_lines(&rows, &no_labels);
    let head = lines.first().ok_or("a window line")?;
    check!(head.starts_with(WINDOW_LINE), "{head}");
    check!(head.contains(" days=2 mint=350 sink=30 net=320"), "{head}");
    check!(
        lines.contains(&"  faucet #1 mint=300".to_owned()),
        "{lines:?}"
    );
    check!(
        lines.contains(&"  faucet #2 mint=50".to_owned()),
        "{lines:?}"
    );
    check!(
        lines.contains(&"  sink #3 intake=30".to_owned()),
        "{lines:?}"
    );
    Ok(())
}

#[test]
fn ex0_the_document_round_trips_and_a_bad_one_restarts() -> R {
    let mut book = CensusBook::default();
    let id = InstrumentId::new("gold").ok_or("instrument id")?;
    let ledger = book.ledgers.entry(id).or_default();
    ledger.advance(reading(&[(1, -1)], 1, 0), 1, 0, 0);
    ledger.advance(reading(&[(1, -2)], 2, 0), 2, 0, 0);
    let kv = vec![(SmolStr::new_static(CENSUS_KEY), book.to_doc())];
    check_eq!(CensusBook::load(&kv), book);
    let garbage = vec![(SmolStr::new_static(CENSUS_KEY), serde_json::json!({"x": 1}))];
    check_eq!(CensusBook::load(&garbage), CensusBook::default());
    let mut other = book.to_doc();
    other["schema"] = serde_json::json!(CENSUS_SCHEMA + 1);
    let future = vec![(SmolStr::new_static(CENSUS_KEY), other)];
    check_eq!(CensusBook::load(&future), CensusBook::default());
    check_eq!(CensusBook::load(&[]), CensusBook::default());
    Ok(())
}

#[test]
fn ex0_the_ic_date_of_a_day() -> R {
    check_eq!(ic_day(86_399), 0);
    check_eq!(ic_day(86_400), 1);
    check_eq!(ic_day(-1), -1);
    check_eq!(date(0), "0-01-01");
    Ok(())
}

// ---- through the world ----------------------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(crate::EconomyModule::with_custody(vec![])),
    ])
}

/// **The test's game**: it declares the census hook through its registration, as tacsim does.
struct CensusGame;

impl mush_core::module::GameModule for CensusGame {
    fn name(&self) -> &'static str {
        "censusgame"
    }

    fn register(
        &mut self,
        reg: &mut mush_core::module::Registrar<'_>,
    ) -> Result<(), mush_core::module::ModError> {
        reg.state(MoneyCensusHook {
            org_classes: ironhold_is_lane,
        });
        Ok(())
    }
}

/// The economy with the test's game beside it.
fn boot_with_game() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(crate::EconomyModule::with_custody(vec![])),
        Box::new(CensusGame),
    ])
}

/// One sweep fire at wall-clock second `t` (the IC clock runs 1:1 in a test world).
fn sweep_at(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1000));
}

/// The test's game names its one organisation a lane institution.
fn ironhold_is_lane(w: &World) -> BTreeMap<OrgId, HoldingClass> {
    w.module_state::<mush_org::OrgStore>()
        .map(|store| {
            store
                .state()
                .orgs()
                .filter(|org| org.name.as_str() == "Ironhold")
                .map(|org| (org.id, HoldingClass::Lane))
                .collect()
        })
        .unwrap_or_default()
}

fn account_where(tw: &TestWorld, pick: impl Fn(&PrincipalId) -> bool) -> Result<AccountId, String> {
    tw.world()
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .snapshot()
        .accounts()
        .find(|a| pick(&a.owner))
        .map(|a| a.id)
        .ok_or_else(|| "no such account".to_owned())
}

#[test]
fn ex0_econ_census_prints_sources_sinks_and_holdings_by_ic_day() -> R {
    let mut tw = boot_with_game();
    let pat = tw.spawn_player("Pat");
    let npc = tw
        .world_mut()
        .create(
            mush_core::ObjKind::Thing,
            "Purse",
            GOD,
            Some(mush_core::LIMBO),
        )
        .map_err(|e| e.to_string())?;
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    ok(&mut tw, GOD, "@account/system levy=gold");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", pat.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", npc.0));
    ok(&mut tw, GOD, "@org/create Ironhold");
    ok(&mut tw, GOD, "@account/open org:Ironhold=gold");
    let system = |name: &'static str| move |owner: &PrincipalId| matches!(owner, PrincipalId::System(id) if id.as_str() == name);
    let faucet = account_where(&tw, system("faucet"))?;
    let levy = account_where(&tw, system("levy"))?;
    let pat_acct = account_where(&tw, |o| *o == PrincipalId::Object(pat))?;
    let npc_acct = account_where(&tw, |o| *o == PrincipalId::Object(npc))?;
    let lane = account_where(&tw, |o| matches!(o, PrincipalId::Organization(_)))?;

    // Day 1: the first sweep opens the census.
    let day = IC_DAY_SECS;
    sweep_at(&mut tw, day + 60);
    // On day 1 the faucet capitalises the lane org and pays two purses; Pat pays a levy.
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={lane} 5000"));
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={pat_acct} 300"));
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={npc_acct} 20"));
    ok(&mut tw, GOD, &format!("@econ/pay {pat_acct}={levy} 40"));
    // Day 2's first sweep closes day 1.
    sweep_at(&mut tw, 2 * day + 60);
    // On day 2 only the levy moves: Pat pays 10 more.
    ok(&mut tw, GOD, &format!("@econ/pay {pat_acct}={levy} 10"));
    sweep_at(&mut tw, 3 * day + 60);

    let out = ok(&mut tw, GOD, "@econ/census");
    let lines: Vec<&str> = out.lines().collect();
    let line = |prefix: String| {
        lines
            .iter()
            .find(|l| l.starts_with(&prefix))
            .copied()
            .ok_or_else(|| format!("no line `{prefix}` in:\n{out}"))
    };
    let day1 = line(format!("{DAY_LINE} {} days=1", date(1)))?;
    check!(
        day1.ends_with("mint=5320 sink=40 net=5280 drift=0"),
        "{day1}"
    );
    let day2 = line(format!("{DAY_LINE} {} days=1", date(2)))?;
    check!(day2.ends_with("mint=0 sink=10 net=-10 drift=0"), "{day2}");
    check!(
        lines.contains(&"  held lane=5000 house=0 org=0 player=250 other=20 total=5270"),
        "{out}"
    );
    check!(
        lines
            .iter()
            .any(|l| *l == format!("  source #{} sys:faucet mint=5320", faucet.0)),
        "{out}"
    );
    check!(
        lines
            .iter()
            .any(|l| *l == format!("  sink #{} sys:levy intake=40", levy.0)),
        "{out}"
    );
    // The open day and the window's totals: the faucet's net mint over both rows.
    check!(
        lines
            .iter()
            .any(|l| l.starts_with(&format!("{DAY_LINE} {} days=0", date(3)))
                && l.ends_with(" open")),
        "{out}"
    );
    let window = line(format!("{WINDOW_LINE} {}..{} days=2", date(1), date(2)))?;
    check!(window.ends_with("mint=5320 sink=50 net=5270"), "{window}");
    check!(
        lines
            .iter()
            .any(|l| *l == format!("  faucet #{} sys:faucet mint=5320", faucet.0)),
        "{out}"
    );
    for l in &lines {
        check!(l.chars().count() <= MAX_COLUMNS, "{l}");
    }
    // One row shown, on request; a bad count and a mortal are refused.
    let one = ok(&mut tw, GOD, "@econ/census 1");
    check!(one.contains("rows=2 shown=1"), "{one}");
    check!(!one.contains(&format!("{DAY_LINE} {} ", date(1))), "{one}");
    check!(err(&mut tw, GOD, "@econ/census 29").contains("Usage: @econ/census"));
    check!(err(&mut tw, pat, "@econ/census").contains("Permission denied"));
    Ok(())
}

#[test]
fn ex0_the_census_survives_a_reboot() -> R {
    let mut tw = boot();
    let pat = tw.spawn_player("Pat");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", pat.0));
    let faucet = account_where(
        &tw,
        |o| matches!(o, PrincipalId::System(id) if id.as_str() == "faucet"),
    )?;
    let pat_acct = account_where(&tw, |o| *o == PrincipalId::Object(pat))?;
    sweep_at(&mut tw, IC_DAY_SECS + 60);
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={pat_acct} 70"));
    sweep_at(&mut tw, 2 * IC_DAY_SECS + 60);
    let before = tw
        .world()
        .module_state::<CensusBook>()
        .cloned()
        .ok_or("the book")?;
    check_eq!(
        before.ledgers.values().next().map(|l| l.rows.len()),
        Some(1)
    );
    // Every document the economy wrote, the `census` one among them, booted again in place
    // over a book wiped in RAM: the boot reads the book back from its document.
    let mut kept = mush_testkit::cold::Boot::new();
    mush_testkit::fold_module_ops_into(&mut kept, tw.take_module_ops());
    check!(
        kept.get(crate::module::NS)
            .is_some_and(|data| data.kv.iter().any(|(key, _)| key == CENSUS_KEY)),
        "the sweep wrote the census document"
    );
    if let Some(book) = tw.world_mut().module_state_mut::<CensusBook>() {
        *book = CensusBook::default();
    }
    tw.try_cold_loaded(kept).map_err(|e| format!("{e:?}"))?;
    check_eq!(tw.world().module_state::<CensusBook>(), Some(&before));
    // And the next day's row opens where the last boot left off: nothing moved, so net 0.
    sweep_at(&mut tw, 3 * IC_DAY_SECS + 60);
    let after = tw
        .world()
        .module_state::<CensusBook>()
        .and_then(|book| book.ledgers.values().next().cloned())
        .ok_or("the ledger")?;
    check_eq!(after.rows.len(), 2);
    check_eq!(
        after.rows.back().map(|row| (row.day, row.net())),
        Some((2, 0))
    );
    Ok(())
}

#[test]
fn ex0_a_count_of_nought_is_refused() -> R {
    let mut tw = boot();
    check!(err(&mut tw, GOD, "@econ/census 0").contains("Usage: @econ/census"));
    check!(err(&mut tw, GOD, "@econ/census seven").contains("Usage: @econ/census"));
    Ok(())
}
