//! **The tenure numbers** (doc 20 §9.2–§9.6 and §11's Tenure and Claims rows; cut SM.6, the
//! brief's lead ruling 6 and decision D9).
//!
//! Industry's tenure seams (`mush_industry::IndustryFamilies::renew_claim`, `lapse_claim`,
//! `take_over_claim`, `transfer_tenure`) are neutral and take every duration as an argument;
//! the numbers are this game's, and live here for SM.10's `+claim/*` to read. SM.6 uses them
//! only in its tests. **No fee constant**: the claim fees are SM.10's to place (lane data or a
//! constant).

/// A claim's term, IC seconds: the dormancy week (§9.2 "term 168 IC days"; §11's Claims row —
/// seven real days at rate 24, the charter's dormancy week, `lane:2171`).
pub const CLAIM_TERM_IC_SECS: i64 = 168 * 86_400;

/// **Use it or lose it** (§9.2; §11's Claims row): a renewal needs this many tonnes worked in
/// the term, so nobody banks ground for a cycle a week.
pub const WORKED_FLOOR_T: u64 = 50;

/// **The cooling term**, IC seconds (§9.2; §11's Tenure row): a lapsed holder, and anyone
/// acting for the same principal, may not stake the block again for one full term from the
/// lapse.
pub const COOLING_TERM_IC_SECS: i64 = 168 * 86_400;

/// **The contest window**, IC seconds (§9.4; §11's Claims row): a rival may contest within 30
/// IC days of a registration.
pub const CONTEST_WINDOW_IC_SECS: i64 = 30 * 86_400;

/// **Live claims per account** (§9.2; §11's Claims row): at most two.
pub const MAX_LIVE_CLAIMS_PER_ACCOUNT: usize = 2;

/// **The Registrar's rule** (§9.6): a papered block worked for at least this many tonnes in the
/// term has the Syndicate paper suspended at its holder's first renewal.
pub const REGISTRAR_WORKED_T: u64 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    /// Doc 20 §9 and §11's numbers, pinned.
    #[test]
    fn sm6_the_tenure_numbers_are_pinned() {
        assert_eq!(CLAIM_TERM_IC_SECS, 14_515_200);
        assert_eq!(WORKED_FLOOR_T, 50);
        assert_eq!(COOLING_TERM_IC_SECS, 14_515_200);
        assert_eq!(CONTEST_WINDOW_IC_SECS, 2_592_000);
        assert_eq!(MAX_LIVE_CLAIMS_PER_ACCOUNT, 2);
        assert_eq!(REGISTRAR_WORKED_T, 100);
    }
}
