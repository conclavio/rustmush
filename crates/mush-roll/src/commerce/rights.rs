//! Operation-specific parent policy and bounded dependent owner terms.
//!
//! These are pure policy components, not capabilities or admission APIs. The
//! registered action must resolve the actual current title, exact qualifying
//! grant/form/revision and Registry status, then recheck them before commit.
//! Passing a parent fence does not establish the requester's own term, authority,
//! capacity or Registry permission. No running sortie consults this module.

use super::{MAX_GRACE_SECONDS, MAX_TERM_SECONDS, TermPhase, TermWindow, WorldIcSeconds};

/// Which operation is being checked, independent of immutable ancestry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentOperation {
    OwnerSeal,
    OwnerFile,
    OwnerSource,
    NewSale,
    PaidRenewal,
    OwnerAttestation,
    ExistingPaidSeal,
    ExistingPaidSource,
}

/// Resolved current rights of this derivative's issuer/owner. Original work has
/// no parent requirement. This value is data and carries no authority token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentRights {
    Original,
    QualifyingGrant,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DependencyError {
    #[error("qualifying parent rights are unavailable")]
    ParentUnavailable,
    #[error("the requested dependent term has no permitted interval")]
    Term,
}

/// Q17's owner restriction and the separate paid-customer ruling. Existing paid
/// grants are checked against their own term/form and Registry status elsewhere;
/// they do not inherit the issuer's current parent-right loss. Immutable parent
/// royalty obligations remain unchanged for every operation.
pub fn check_parent(
    operation: ParentOperation,
    rights: ParentRights,
) -> Result<(), DependencyError> {
    match (operation, rights) {
        (ParentOperation::ExistingPaidSeal | ParentOperation::ExistingPaidSource, _)
        | (_, ParentRights::Original | ParentRights::QualifyingGrant) => Ok(()),
        (_, ParentRights::Unavailable) => Err(DependencyError::ParentUnavailable),
    }
}

/// Construct a newly attested dependent owner's window from a resolved current
/// parent window. Call only after the exact qualifying grant has been verified.
/// This never mutates an existing child term or extends one on restoration.
///
/// An issue during parent Grace retains the parent's real eligibility origin and
/// end, with the actual current issue time. It cannot manufacture an Active term
/// or pretend the child was issued earlier. The child receives at most its own
/// configured Grace length, always bounded by the parent.
pub fn dependent_owner_window(
    now: WorldIcSeconds,
    term_seconds: i64,
    grace_seconds: i64,
    parent: TermWindow,
) -> Result<TermWindow, DependencyError> {
    if !(1..=MAX_TERM_SECONDS).contains(&term_seconds)
        || !(0..=MAX_GRACE_SECONDS).contains(&grace_seconds)
    {
        return Err(DependencyError::Term);
    }
    let (origin, term_end) = match parent.phase_at(now) {
        TermPhase::Active => {
            let requested_end = now
                .0
                .checked_add(term_seconds)
                .ok_or(DependencyError::Term)?;
            (now, WorldIcSeconds(requested_end.min(parent.term_end().0)))
        }
        TermPhase::Grace => (parent.eligibility_start(), parent.term_end()),
        TermPhase::NotEffective | TermPhase::Lapsed => {
            return Err(DependencyError::ParentUnavailable);
        }
    };
    let grace_end = WorldIcSeconds(
        term_end
            .0
            .checked_add(grace_seconds)
            .ok_or(DependencyError::Term)?
            .min(parent.grace_end().0),
    );
    TermWindow::new(origin, now, term_end, grace_end).map_err(|_| DependencyError::Term)
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn window(
        origin: i64,
        issued: i64,
        end: i64,
        grace: i64,
    ) -> Result<TermWindow, super::super::ValueError> {
        TermWindow::new(
            WorldIcSeconds(origin),
            WorldIcSeconds(issued),
            WorldIcSeconds(end),
            WorldIcSeconds(grace),
        )
    }

    fn require(ok: bool, message: &'static str) -> TestResult {
        if ok { Ok(()) } else { Err(message.into()) }
    }

    #[test]
    fn parent_loss_blocks_ada_but_preserves_bos_already_paid_access() -> TestResult {
        use ParentOperation as Op;
        let lost = ParentRights::Unavailable;
        for op in [
            Op::OwnerSeal,
            Op::OwnerFile,
            Op::OwnerSource,
            Op::NewSale,
            Op::PaidRenewal,
            Op::OwnerAttestation,
        ] {
            require(
                check_parent(op, lost) == Err(DependencyError::ParentUnavailable),
                "dependent owner's operation remained open",
            )?;
        }
        check_parent(Op::ExistingPaidSeal, lost)?;
        check_parent(Op::ExistingPaidSource, lost)?;
        // Bo's fork is now his own derivative. Its immediate parent is Bo's
        // paid Child grant, not Ada's unavailable ancestor grant.
        check_parent(Op::OwnerFile, ParentRights::QualifyingGrant)?;
        require(
            check_parent(Op::OwnerFile, lost).is_err(),
            "Bo could file after losing his own immediate-parent grant",
        )
    }

    #[test]
    fn restoration_reopens_parent_fence_without_extending_any_term() -> TestResult {
        let child = window(0, 0, 100, 110)?;
        check_parent(ParentOperation::OwnerSeal, ParentRights::QualifyingGrant)?;
        require(
            child.phase_at(WorldIcSeconds(110)) == TermPhase::Lapsed,
            "restoration extended the old child term",
        )?;
        check_parent(ParentOperation::ExistingPaidSeal, ParentRights::Unavailable)?;
        require(
            child.phase_at(WorldIcSeconds(110)) == TermPhase::Lapsed,
            "paid exception extended the customer's term",
        )
    }

    #[test]
    fn shorter_parent_caps_active_term_and_grace() -> TestResult {
        let parent = window(0, 0, 100, 115)?;
        let child = dependent_owner_window(WorldIcSeconds(50), 90, 30, parent)?;
        require(child.issued_at() == WorldIcSeconds(50), "wrong issue date")?;
        require(
            child.term_end() == WorldIcSeconds(100) && child.grace_end() == WorldIcSeconds(115),
            "child outran parent",
        )?;
        require(
            child.phase_at(WorldIcSeconds(100)) == TermPhase::Grace,
            "exact end must be Grace",
        )?;
        require(
            child.phase_at(WorldIcSeconds(115)) == TermPhase::Lapsed,
            "exact Grace end must lapse",
        )
    }

    #[test]
    fn child_can_choose_shorter_term_without_inheriting_all_parent_grace() -> TestResult {
        let parent = window(0, 0, 1000, 1200)?;
        let child = dependent_owner_window(WorldIcSeconds(50), 100, 20, parent)?;
        require(
            child.term_end() == WorldIcSeconds(150) && child.grace_end() == WorldIcSeconds(170),
            "own interval was extended to parent interval",
        )
    }

    #[test]
    fn filing_during_parent_grace_preserves_real_dates() -> TestResult {
        let parent = window(0, 0, 100, 130)?;
        let child = dependent_owner_window(WorldIcSeconds(105), 90, 20, parent)?;
        require(
            child.eligibility_start() == WorldIcSeconds(0)
                && child.issued_at() == WorldIcSeconds(105),
            "fabricated issue or eligibility origin",
        )?;
        require(
            child.term_end() == WorldIcSeconds(100) && child.grace_end() == WorldIcSeconds(120),
            "fabricated Active interval",
        )?;
        require(
            child.phase_at(WorldIcSeconds(104)) == TermPhase::NotEffective
                && child.phase_at(WorldIcSeconds(105)) == TermPhase::Grace,
            "child became eligible before it was issued",
        )?;
        require(
            dependent_owner_window(WorldIcSeconds(125), 90, 20, parent)
                == Err(DependencyError::Term),
            "issue without remaining child Grace succeeded",
        )
    }

    #[test]
    fn future_lapsed_and_zero_remaining_parent_rights_refuse() -> TestResult {
        let parent = window(100, 90, 200, 220)?;
        for now in [89, 90, 99, 220, 221] {
            require(
                dependent_owner_window(WorldIcSeconds(now), 50, 10, parent)
                    == Err(DependencyError::ParentUnavailable),
                "unavailable parent supplied a child term",
            )?;
        }
        require(
            dependent_owner_window(WorldIcSeconds(205), 50, 0, parent)
                == Err(DependencyError::Term),
            "zero Grace admitted a Grace issue",
        )
    }

    #[test]
    fn duration_and_clock_overflow_are_refused() -> TestResult {
        let parent = window(0, 0, 100, 120)?;
        for (term, grace) in [
            (0, 1),
            (MAX_TERM_SECONDS + 1, 1),
            (1, -1),
            (1, MAX_GRACE_SECONDS + 1),
        ] {
            require(
                dependent_owner_window(WorldIcSeconds(1), term, grace, parent)
                    == Err(DependencyError::Term),
                "invalid configured duration admitted",
            )?;
        }
        let extreme = window(i64::MAX - 10, i64::MAX - 10, i64::MAX - 1, i64::MAX)?;
        require(
            dependent_owner_window(WorldIcSeconds(i64::MAX - 5), 100, 1, extreme)
                == Err(DependencyError::Term),
            "clock overflow was accepted",
        )
    }
}
