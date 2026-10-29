//! Doc 17h §3 M8f P1: the rating score over hand-built result pages.
use super::super::{
    Applicability, CaseBinding, CaseKey, CaseKind, CaseOutcome, CaseResult, Coverage, MeterSummary,
    PageContent, Peak, ProbeName, RatingClass, RatingError, TrialEnd, TrialLayout, TrialOutcome,
};
use super::{RatingScore, RatingTally, rating_score, score_per_credit};
use mush_core::mechanics::DefinitionDigest;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn hash(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}
fn trial(variant: u8, seed: u8) -> CaseKind {
    CaseKind::Trial {
        variant,
        layout: TrialLayout::Duel,
        seed,
        swapped: false,
    }
}
fn coverage() -> Result<Coverage, RatingError> {
    Coverage::new(Applicability::Armed, 1, 1, hash(8))
}
/// A conformant case; a trial ends `outcome`.
fn row(
    class: RatingClass,
    kind: CaseKind,
    outcome: TrialOutcome,
) -> Result<CaseResult, RatingError> {
    CaseResult::new(
        CaseKey::new(class, kind)?,
        CaseBinding::new(hash(2), hash(4), hash(6), hash(7)),
        MeterSummary::new(1, 10, 1, Some(Peak::new(10, 1, 0, 0)?), 0, 0, 0, 0)?,
        coverage()?,
        0,
        CaseOutcome::Conformant {},
        matches!(kind, CaseKind::Trial { .. })
            .then(|| TrialEnd::new(1000, 42, hash(9), outcome))
            .transpose()?,
    )
}
/// A trial over its ops budget: no terminal.
fn over(class: RatingClass, kind: CaseKind) -> Result<CaseResult, RatingError> {
    CaseResult::new(
        CaseKey::new(class, kind)?,
        CaseBinding::new(hash(2), hash(4), hash(6), hash(7)),
        MeterSummary::new(1, 10, 1, Some(Peak::new(10, 1, 0, 0)?), 1, 0, 0, 0)?,
        coverage()?,
        0,
        CaseOutcome::BudgetExceeded {},
        None,
    )
}
fn probe(class: RatingClass) -> Result<CaseResult, RatingError> {
    row(
        class,
        CaseKind::Probe {
            pair: 0,
            name: ProbeName::EmptyArmed,
        },
        TrialOutcome::Win,
    )
}
fn page(ordinal: u16, rows: Vec<CaseResult>) -> Result<PageContent, RatingError> {
    PageContent::new(hash(3), hash(10), ordinal, rows)
}

#[test]
fn every_trial_outcome_arm_counts_only_a_win() -> TestResult {
    // Keys ascend (a page's rule): the trials sort before the probe.
    let rows = vec![
        row(RatingClass::I, trial(0, 1), TrialOutcome::Win)?,
        row(RatingClass::I, trial(0, 2), TrialOutcome::Loss)?,
        row(RatingClass::I, trial(1, 1), TrialOutcome::Draw)?,
        row(RatingClass::I, trial(1, 2), TrialOutcome::Timeout)?,
        probe(RatingClass::I)?,
    ];
    check_eq!(
        rating_score(&[page(0, rows)?]),
        Some(RatingScore {
            class: RatingClass::I,
            wins: 1,
            cases: 4,
            permille: 250,
        }),
        "the probe is no trial; a Loss, a Draw and a Timeout are no wins"
    );
    Ok(())
}

#[test]
fn the_lowest_passing_class_is_scored_across_pages() -> TestResult {
    // Class I fails its budget; II passes with 2 of 3 wins; III passes too.
    let first = page(
        0,
        vec![
            row(RatingClass::I, trial(0, 1), TrialOutcome::Win)?,
            over(RatingClass::I, trial(0, 2))?,
            row(RatingClass::II, trial(0, 1), TrialOutcome::Win)?,
        ],
    )?;
    let second = page(
        1,
        vec![
            row(RatingClass::II, trial(0, 2), TrialOutcome::Win)?,
            row(RatingClass::II, trial(1, 1), TrialOutcome::Loss)?,
            row(RatingClass::III, trial(0, 1), TrialOutcome::Loss)?,
        ],
    )?;
    check_eq!(
        rating_score(&[first, second]),
        Some(RatingScore {
            class: RatingClass::II,
            wins: 2,
            cases: 3,
            permille: 666,
        })
    );
    Ok(())
}

#[test]
fn no_passing_class_no_trials_or_no_pages_is_unscored() -> TestResult {
    check_eq!(rating_score(&[]), None, "no pages");
    let failing = page(
        0,
        vec![
            row(RatingClass::I, trial(0, 1), TrialOutcome::Win)?,
            over(RatingClass::I, trial(0, 2))?,
        ],
    )?;
    check_eq!(rating_score(&[failing]), None, "a failing-only revision");
    let probes = page(0, vec![probe(RatingClass::I)?])?;
    check_eq!(rating_score(&[probes]), None, "zero trial cases");
    Ok(())
}

#[test]
fn a_trial_without_terminal_is_no_win() -> TestResult {
    // A budget-faulted trial carries no terminal; once its class fails the
    // next class is scored, and the faulted row is never a win anywhere.
    let rows = vec![
        over(RatingClass::I, trial(0, 1))?,
        row(RatingClass::II, trial(0, 1), TrialOutcome::Win)?,
        row(RatingClass::II, trial(0, 2), TrialOutcome::Timeout)?,
    ];
    let score = rating_score(&[page(0, rows)?]).ok_or("scored")?;
    check_eq!(
        (score.class, score.wins, score.cases, score.permille),
        (RatingClass::II, 1, 2, 500)
    );
    Ok(())
}

#[test]
fn score_per_credit_is_checked_and_refuses_a_free_line() -> TestResult {
    check_eq!(score_per_credit(500, 40_000), Some(12));
    check_eq!(score_per_credit(1000, 1), Some(1_000_000));
    check_eq!(
        score_per_credit(1000, 0),
        None,
        "a zero price is unrankable"
    );
    check_eq!(
        score_per_credit(1000, -5),
        None,
        "a negative price is unrankable"
    );
    check_eq!(score_per_credit(0, 40_000), Some(0));
    Ok(())
}

#[test]
fn tallies_of_disjoint_reads_merge_to_the_whole() -> TestResult {
    let first = page(
        0,
        vec![
            row(RatingClass::I, trial(0, 1), TrialOutcome::Win)?,
            row(RatingClass::I, trial(0, 2), TrialOutcome::Loss)?,
        ],
    )?;
    let second = page(
        1,
        vec![
            row(RatingClass::I, trial(1, 1), TrialOutcome::Win)?,
            row(RatingClass::I, trial(1, 2), TrialOutcome::Win)?,
        ],
    )?;
    let a = RatingTally::default().add_page(&first).ok_or("tally")?;
    let b = RatingTally::default().add_page(&second).ok_or("tally")?;
    let merged = b.merge(a).ok_or("merge")?;
    check_eq!(merged.score(), rating_score(&[first, second]));
    check_eq!(merged.score().map(|s| s.permille), Some(750));
    check_eq!(RatingTally::default().score(), None, "nothing read");
    Ok(())
}
