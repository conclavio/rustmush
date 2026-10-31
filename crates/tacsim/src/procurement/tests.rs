//! Doc 17h §3 M8f: the procurement core, pure (P1, P4–P7). The composed purchase runs in
//! `minds/bulk/entitlement_integration_tests.rs`'s rig (`procurement_*`).
use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CommitToken, ObjId};
use mush_market::mind_commands::ListingForm;
use mush_org::OrgId;
use mush_roll::commerce::{CharterId, LicenceId};

use super::ledger::{Period, ProcurementRow, RowPhase};
use super::rank::{self, Candidate};
use crate::launch::ProcurementRole;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn charter(n: u32) -> Result<CharterId, Box<dyn std::error::Error>> {
    Ok(CharterId::from_object(ObjId(n))?)
}
fn row(
    period: Period,
    op: u8,
    (charter, licence): (CharterId, u64),
    price_minor: i64,
    phase: RowPhase,
) -> Result<ProcurementRow, Box<dyn std::error::Error>> {
    Ok(ProcurementRow {
        format: 1,
        org: OrgId(3),
        period,
        ordinal: u32::from(op),
        operation: token(op),
        listing: token(100),
        charter,
        licence: LicenceId::new(licence)?,
        price_minor,
        phase,
        agent: ObjId(50),
    })
}
const MAY: Period = Period {
    year: 2350,
    month: 5,
};
const JUNE: Period = Period {
    year: 2350,
    month: 6,
};

#[test]
fn the_period_is_the_civil_month() -> TestResult {
    let day = 86_400_i64;
    check_eq!(Period::of_ic(0), Period { year: 0, month: 1 });
    // The game calendar's January has 31 days; a year has 365.
    check_eq!(Period::of_ic(30 * day).month, 1);
    check_eq!(Period::of_ic(31 * day), Period { year: 0, month: 2 });
    check_eq!(Period::of_ic(364 * day), Period { year: 0, month: 12 });
    check_eq!(Period::of_ic(365 * day), Period { year: 1, month: 1 });
    check!(
        Period::of_ic(364 * day) < Period::of_ic(365 * day),
        "months order by year first"
    );
    check_eq!(JUNE.previous(), MAY);
    check_eq!(
        Period {
            year: 2351,
            month: 1
        }
        .previous(),
        Period {
            year: 2350,
            month: 12
        }
    );
    Ok(())
}

#[test]
fn the_cap_is_hulls_per_charter_floored() -> TestResult {
    check_eq!(
        rank::cap_for(400, 10),
        4,
        "ten hulls at 400 per mille: four"
    );
    check_eq!(
        rank::cap_for(400, 2),
        0,
        "two hulls: no seat for any one charter"
    );
    check_eq!(rank::cap_for(400, 3), 1);
    check_eq!(rank::cap_for(1000, 7), 7);
    check_eq!(rank::cap_for(1, 0), 0);
    Ok(())
}

#[test]
fn the_month_spend_counts_its_own_unrefunded_rows() -> TestResult {
    let c = charter(10)?;
    let rows = vec![
        row(MAY, 1, (c, 1), 40_000, RowPhase::Granted)?,
        row(MAY, 2, (c, 2), 40_000, RowPhase::Funded)?,
        row(MAY, 3, (c, 3), 40_000, RowPhase::Refunded)?,
        row(JUNE, 4, (c, 4), 40_000, RowPhase::Granted)?,
    ];
    check_eq!(rank::spent(&rows, MAY), Some(80_000));
    check_eq!(rank::spent(&rows, JUNE), Some(40_000));
    check_eq!(rank::spent(&[], MAY), Some(0));
    Ok(())
}

#[test]
fn held_counts_the_roll_and_funded_rows_each_licence_once() -> TestResult {
    let (a, b) = (charter(10)?, charter(11)?);
    let holdings = vec![(LicenceId::new(1)?, a), (LicenceId::new(2)?, a)];
    let rows = vec![
        // Granted and on the Roll: counted once.
        row(MAY, 1, (a, 1), 40_000, RowPhase::Granted)?,
        // Granted, lapsed off the Roll: no seat.
        row(MAY, 5, (b, 5), 40_000, RowPhase::Granted)?,
        // Funded, not yet on the Roll: a seat.
        row(MAY, 3, (b, 3), 40_000, RowPhase::Funded)?,
        // Refunded: nothing.
        row(MAY, 4, (b, 4), 40_000, RowPhase::Refunded)?,
    ];
    let held = rank::held_per_charter(&holdings, &rows);
    check_eq!(held.get(&a).copied(), Some(2));
    check_eq!(held.get(&b).copied(), Some(1));
    Ok(())
}

#[test]
fn roles_admit_only_their_forms() -> TestResult {
    let all = ProcurementRole::all();
    for form in [
        ListingForm::Sealed,
        ListingForm::Open,
        ListingForm::OpenResale,
    ] {
        check!(rank::admits(&all, form));
    }
    let sealed = [ProcurementRole::Sealed];
    check!(rank::admits(&sealed, ListingForm::Sealed));
    check!(!rank::admits(&sealed, ListingForm::Open));
    check!(!rank::admits(&sealed, ListingForm::OpenResale));
    Ok(())
}

/// The three listed lines of the zero-PC story's first month, with distinct score per credit:
/// A 900‰ at 400cr (2.25‰ a credit), B 800‰ at 400cr (2.0), C 900‰ at 900cr (1.0).
fn lines() -> Result<[Candidate; 3], Box<dyn std::error::Error>> {
    Ok([
        Candidate {
            listing: token(3),
            charter: charter(12)?,
            price_minor: 90_000,
            permille: 900,
        },
        Candidate {
            listing: token(2),
            charter: charter(11)?,
            price_minor: 40_000,
            permille: 800,
        },
        Candidate {
            listing: token(1),
            charter: charter(10)?,
            price_minor: 40_000,
            permille: 900,
        },
    ])
}

#[test]
fn the_best_per_credit_wins_and_ties_break_totally() -> TestResult {
    let lines = lines()?;
    let none = BTreeSet::new();
    let held = BTreeMap::new();
    let best = rank::choose(&lines, 200_000, &held, 4, &none).ok_or("a choice")?;
    check_eq!(best.listing, token(1), "A: the most score per credit");
    // Over budget: only what is affordable.
    let best = rank::choose(&lines, 39_999, &held, 4, &none);
    check!(best.is_none(), "nothing fits a budget under every price");
    // A skipped listing is passed over.
    let skipped = BTreeSet::from([token(1)]);
    let best = rank::choose(&lines, 200_000, &held, 4, &skipped).ok_or("a choice")?;
    check_eq!(best.listing, token(2));
    // A free line is unrankable (P1) and never bought.
    let free = [Candidate {
        listing: token(9),
        charter: charter(19)?,
        price_minor: 0,
        permille: 1000,
    }];
    check!(rank::choose(&free, 200_000, &held, 4, &none).is_none());
    // Equal per credit: the higher score wins (900‰ at 900 against 400‰ at 400, both 1,000).
    let by_score = [
        Candidate {
            listing: token(5),
            charter: charter(15)?,
            price_minor: 400,
            permille: 400,
        },
        Candidate {
            listing: token(4),
            charter: charter(14)?,
            price_minor: 900,
            permille: 900,
        },
    ];
    check_eq!(
        rank::choose(&by_score, 200_000, &held, 4, &none).map(|c| c.listing),
        Some(token(4))
    );
    // Per credit is exact: 100‰ at 3,001 and at 3,000 floored to the same truncated key (33),
    // yet the cheaper is better per credit and wins on that key alone.
    let by_price = [
        Candidate {
            listing: token(8),
            charter: charter(18)?,
            price_minor: 3_001,
            permille: 100,
        },
        Candidate {
            listing: token(9),
            charter: charter(19)?,
            price_minor: 3_000,
            permille: 100,
        },
    ];
    check_eq!(
        rank::choose(&by_price, 200_000, &held, 4, &none).map(|c| c.listing),
        Some(token(9))
    );
    // Equal per credit and score (two 0‰ lines, admitted only by a test seam): the cheaper.
    let zero = [
        Candidate {
            listing: token(8),
            charter: charter(18)?,
            price_minor: 3_001,
            permille: 0,
        },
        Candidate {
            listing: token(9),
            charter: charter(19)?,
            price_minor: 5_000,
            permille: 0,
        },
    ];
    check_eq!(
        rank::choose(&zero, 200_000, &held, 4, &none).map(|c| c.listing),
        Some(token(8))
    );
    // Equal per credit: the higher score, then the cheaper, then the token.
    let tied = [
        Candidate {
            listing: token(7),
            charter: charter(17)?,
            price_minor: 40_000,
            permille: 400,
        },
        Candidate {
            listing: token(6),
            charter: charter(16)?,
            price_minor: 40_000,
            permille: 400,
        },
    ];
    check_eq!(
        rank::choose(&tied, 200_000, &held, 4, &none).map(|c| c.listing),
        Some(token(6))
    );
    Ok(())
}

/// **The M8-1 review finding**: under the default 2,000cr budget the old truncated key
/// `permille × 1000 / price_minor` read 5 for both of these, so the higher score won the tie,
/// though B is 9% better per credit (550‰ at 1,000cr against 1000‰ at 1,990cr). The exact
/// comparison buys B.
#[test]
fn per_credit_is_exact_where_truncation_tied() -> TestResult {
    let lines = [
        Candidate {
            listing: token(1),
            charter: charter(10)?,
            price_minor: 199_000,
            permille: 1000,
        },
        Candidate {
            listing: token(2),
            charter: charter(11)?,
            price_minor: 100_000,
            permille: 550,
        },
    ];
    check_eq!(
        (1000 * 1000 / 199_000, 550 * 1000 / 100_000),
        (5, 5),
        "the truncated keys tie"
    );
    let none = BTreeSet::new();
    let held = BTreeMap::new();
    check_eq!(
        rank::choose(&lines, 200_000, &held, 4, &none).map(|c| c.listing),
        Some(token(2)),
        "B, the better per credit"
    );
    // The order is the same at any scale: a 1,000,000cr budget over lines priced in the
    // hundreds of thousands, where the truncated key was zero for both.
    let dear = [
        Candidate {
            price_minor: 99_000_000,
            ..lines[0]
        },
        Candidate {
            price_minor: 50_000_000,
            ..lines[1]
        },
    ];
    check_eq!(
        rank::choose(&dear, 100_000_000, &held, 4, &none).map(|c| c.listing),
        Some(token(2))
    );
    Ok(())
}

/// **The zero-PC story's first half, pure** (brief test 3's decision logic): one IC month, a
/// House of ten titled hulls, a 2,000cr budget, the three lines above and a fourth, unlisted
/// (never in the candidates: the tick reads listings only). Buying until nothing is chosen:
/// four licences of A (the cap: ten hulls at 400‰), then B with what is left; C (worst per
/// credit) untouched; the month's spend within the budget. A second month whose budget is spent
/// buys nothing; a fresh month buys again only under the cap the held licences leave.
#[test]
fn one_month_buys_the_best_two_under_the_cap_within_budget() -> TestResult {
    let lines = lines()?;
    let budget = 200_000_i64;
    let cap = rank::cap_for(400, 10);
    let none = BTreeSet::new();
    let mut rows: Vec<ProcurementRow> = Vec::new();
    let mut licence = 0_u64;
    loop {
        let spent = rank::spent(&rows, MAY).ok_or("spend")?;
        let held = rank::held_per_charter(&[], &rows);
        let Some(best) = rank::choose(&lines, budget - spent, &held, cap, &none).copied() else {
            break;
        };
        licence += 1;
        let op = u8::try_from(licence)?;
        rows.push(row(
            MAY,
            op,
            (best.charter, licence),
            best.price_minor,
            RowPhase::Funded,
        )?);
        check!(rows.len() <= 10, "the loop ends");
    }
    let held = rank::held_per_charter(&[], &rows);
    let (a, b, c) = (lines[2].charter, lines[1].charter, lines[0].charter);
    check_eq!(
        held.get(&a).copied(),
        Some(4),
        "A at its cap: four of ten hulls"
    );
    check_eq!(held.get(&b).copied(), Some(1), "B with the budget left");
    check_eq!(
        held.get(&c).copied(),
        None,
        "C, the worst per credit, untouched"
    );
    check_eq!(rank::spent(&rows, MAY), Some(200_000), "exactly the budget");
    // (A second month at a spent budget is the composed test's, `procurement_tick`.)
    // A fresh June, the May licences granted and held: A stays at its cap, B is chosen.
    let granted: Vec<(LicenceId, CharterId)> =
        rows.iter().map(|r| (r.licence, r.charter)).collect();
    let held = rank::held_per_charter(&granted, &[]);
    let best = rank::choose(&lines, budget, &held, cap, &none).ok_or("June's choice")?;
    check_eq!(best.listing, lines[1].listing, "A is capped, so B");
    Ok(())
}

#[test]
fn rows_check_their_bounds_and_change_only_phase() -> TestResult {
    let r = row(MAY, 1, (charter(10)?, 1), 40_000, RowPhase::Funded)?;
    r.check()?;
    check!(
        ProcurementRow {
            price_minor: 0,
            ..r.clone()
        }
        .check()
        .is_err()
    );
    check!(
        ProcurementRow {
            period: Period {
                year: 2350,
                month: 13
            },
            ..r.clone()
        }
        .check()
        .is_err()
    );
    check!(
        ProcurementRow {
            agent: mush_core::GOD,
            ..r.clone()
        }
        .check()
        .is_err()
    );
    let granted = r.with_phase(RowPhase::Granted);
    check_eq!(granted.with_phase(RowPhase::Funded), r);
    check!(granted.counts() && !r.with_phase(RowPhase::Refunded).counts());
    check_eq!(
        r.key().key_text(),
        "00000000000000000003/2350/05/000001",
        "the organisation, the month, the ordinal"
    );
    check!(
        r.key()
            .key_text()
            .starts_with(&super::ledger::ProcurementKey::period_prefix(OrgId(3), MAY))
    );
    Ok(())
}

#[test]
fn a_full_month_closes_at_the_read_bound() -> TestResult {
    let c = charter(10)?;
    let rows = vec![
        row(MAY, 1, (c, 1), 100, RowPhase::Granted)?,
        row(MAY, 2, (c, 2), 100, RowPhase::Refunded)?,
        row(JUNE, 3, (c, 3), 100, RowPhase::Granted)?,
    ];
    check!(
        rank::ledger_full(&rows, MAY, 2),
        "refunded rows fill a month too"
    );
    check!(!rank::ledger_full(&rows, MAY, 3));
    check!(
        !rank::ledger_full(&rows, JUNE, 2),
        "another month's rows count apart"
    );
    Ok(())
}

#[test]
fn the_policy_bounds_and_defaults() -> TestResult {
    let defaults = crate::config::ProcurementConfig::default();
    check_eq!(
        defaults.budget_minor_per_ic_month,
        200_000,
        "Q-M8-6: 2,000cr"
    );
    check_eq!(defaults.cap_permille, 400, "D-UM-54: 400 per mille");
    defaults.validate()?;
    check!(
        crate::config::ProcurementConfig {
            cap_permille: 0,
            ..defaults
        }
        .validate()
        .is_err()
    );
    check!(
        crate::config::ProcurementConfig {
            budget_minor_per_ic_month: 0,
            ..defaults
        }
        .validate()
        .is_err()
    );
    let policy: crate::launch::ProcurementPolicy =
        serde_json::from_value(serde_json::json!({"org": 3, "scope": 4, "agent": 50}))?;
    check!(policy.holds());
    check_eq!(
        policy.roles,
        ProcurementRole::all(),
        "every form by default"
    );
    check_eq!(policy.budget(&defaults), 200_000);
    check_eq!(policy.cap(&defaults), 400);
    let own: crate::launch::ProcurementPolicy = serde_json::from_value(serde_json::json!({
        "org": 3, "scope": 4, "agent": 50, "budget_minor_per_ic_month": 5, "cap_permille": 1000,
        "roles": ["sealed"]
    }))?;
    check_eq!((own.budget(&defaults), own.cap(&defaults)), (5, 1000));
    let ceiling: crate::launch::ProcurementPolicy = serde_json::from_value(serde_json::json!({
        "org": 3, "scope": 4, "agent": 50, "budget_minor_per_ic_month": 100_000_000
    }))?;
    check!(ceiling.holds(), "the ceiling itself is admitted");
    for bad in [
        serde_json::json!({"org": 3, "scope": 4, "agent": 0}),
        serde_json::json!({"org": 3, "scope": 4, "agent": 1}),
        serde_json::json!({"org": 3, "scope": 4, "agent": 50, "cap_permille": 0}),
        serde_json::json!({"org": 3, "scope": 4, "agent": 50, "cap_permille": 1001}),
        serde_json::json!({"org": 3, "scope": 4, "agent": 50, "budget_minor_per_ic_month": 0}),
        serde_json::json!({"org": 3, "scope": 4, "agent": 50, "budget_minor_per_ic_month": -5}),
        serde_json::json!({"org": 3, "scope": 4, "agent": 50, "budget_minor_per_ic_month": 100_000_001}),
        serde_json::json!({"org": 3, "scope": 4, "agent": 50, "roles": []}),
        serde_json::json!({"org": 3, "scope": 4, "agent": 50, "roles": ["open", "open"]}),
    ] {
        let policy: crate::launch::ProcurementPolicy = serde_json::from_value(bad.clone())?;
        check!(!policy.holds(), "{bad}");
    }
    check!(
        serde_json::from_value::<crate::launch::ProcurementPolicy>(
            serde_json::json!({"org": 3, "scope": 4, "agent": 50, "extra": 1})
        )
        .is_err(),
        "strict"
    );
    Ok(())
}
