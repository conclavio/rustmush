//! **The import duty at sale** (doc 50 cut OP.5; Q-50-29, Q-50-59, Q-50-64).
//!
//! When a good on the schedule's list is sold at a venue whose place's de facto holder levies an
//! import duty, into a bid or from a listing, the duty comes out of the seller's proceeds: one cut
//! of the proceeds policy ([`crate::port::proceeds`]), paid to the holder's terms row's payee
//! (Q-50-64: in the first operation only the duty follows the holder). Cargo landed and stored
//! pays nothing until it is sold there.
//!
//! - **The rate** is the row's `import_duty_permille` of the goods' catalog value
//!   (`reference_major`, Q-50-29: of reference value), never more than the sale's gross.
//! - **The list** is the row's `import_goods`. A good off it, or a holder whose row levies none
//!   (the Freeport at Isidis today), pays nothing.
//! - **The line** names the duty with its reason, the holder and since when, as the transit
//!   duty's does (`crate::space::duty::reason_clause`).

use mush_core::World;
use mush_economy::{AccountId, Amount};
use mush_market::{ProceedsCut, SaleTerms};
use smol_str::SmolStr;

/// The cut's label on the sale's payout rows.
pub const DUTY_LABEL: &str = "import duty";
/// The clause a sale's line gains (`{duty}`, `{percent}`, `{reason}`).
pub const DUTY_CLAUSE: &str = " Import duty {duty} ({percent} of the catalog value; {reason}).";

/// **One sale's import duty**: the amount, who is paid, and the clause its line gains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportDuty {
    /// The payee's credit account.
    pub to: AccountId,
    /// The duty, minor units (positive).
    pub amount: Amount,
    /// The line's clause.
    pub clause: String,
}

impl ImportDuty {
    /// The proceeds cut.
    #[must_use]
    pub fn cut(&self) -> ProceedsCut {
        ProceedsCut {
            to: self.to,
            amount: self.amount,
            label: SmolStr::new_static(DUTY_LABEL),
        }
    }
}

/// **The import duty `sale` owes**, or `None` when its venue's holder levies none on its good.
#[must_use]
pub fn import_duty(w: &World, sale: &SaleTerms<'_>) -> Option<ImportDuty> {
    let (def, _) = crate::port::market::port_of_venue(w, sale.market)?;
    let terms = crate::polity::terms_at(w, &def.location)?;
    let row = terms.row.as_ref()?;
    if row.import_duty_permille == 0 || !row.import_goods.contains(sale.commodity) {
        return None;
    }
    let reference = w
        .module_state::<crate::TacsimStore>()?
        .goods()?
        .good(sale.commodity)?
        .reference_major;
    let scale = 10_i128.pow(u32::from(crate::port::apply::credit_scale(w)));
    let value = i128::from(reference) * i128::from(sale.quantity) * scale;
    let duty =
        (value * i128::from(row.import_duty_permille) / 1_000).min(i128::from(sale.gross.minor()));
    let duty = i64::try_from(duty).ok().filter(|d| *d > 0)?;
    let (_, to) = crate::polity::lane_org_account(w, &row.payee)?;
    let amount = Amount::from_minor(duty);
    let (whole, tenth) = (row.import_duty_permille / 10, row.import_duty_permille % 10);
    let percent = if tenth == 0 {
        format!("{whole}%")
    } else {
        format!("{whole}.{tenth}%")
    };
    let clause = DUTY_CLAUSE
        .replace("{duty}", &crate::port::board::money(w, amount))
        .replace("{percent}", &percent)
        .replace(
            "{reason}",
            &crate::space::duty::reason_clause(w, &terms).unwrap_or_default(),
        );
    Some(ImportDuty { to, amount, clause })
}
