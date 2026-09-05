//! The **one** player-facing amount render (E6a step 1, E2-C16 / 08g §12.1 item 4).
//!
//! Every place the economy — or a module layered on it — prints an amount for a
//! person goes through [`render_amount`]: scale-aware (the instrument's `scale`
//! decides the decimal places, exactly as `parse_amount` reads them), symbol-aware
//! (an [`crate::InstrumentDisplay`] puts `¥` before or `€` after the digits), and
//! sign-correct (the minus sign comes before a prefix symbol: `-¥5`, never `¥-5`).
//! The look-back found five renders printing raw minor units against the
//! instrument's own rule (08g §3); one function means there is no sixth.

use crate::amount::Amount;
use crate::ids::{AccountId, InstrumentId};
use crate::record::SymbolPosition;
use crate::snapshot::EconomySnapshot;

/// Render `amount` in `instrument`'s scale and symbol. An unknown instrument
/// renders the bare minor value (scale 0, no symbol) — a render never refuses.
#[must_use]
pub fn render_amount(
    snapshot: &EconomySnapshot,
    instrument: &InstrumentId,
    amount: Amount,
) -> String {
    let Some(def) = snapshot.instrument(instrument) else {
        return amount.to_decimal_string(0);
    };
    let digits = amount.to_decimal_string(def.scale);
    let Some(display) = &def.display else {
        return digits;
    };
    match display.position {
        SymbolPosition::Prefix => match digits.strip_prefix('-') {
            Some(rest) => format!("-{}{rest}", display.symbol),
            None => format!("{}{digits}", display.symbol),
        },
        SymbolPosition::Suffix => format!("{digits}{}", display.symbol),
    }
}

/// [`render_amount`] in the instrument **`account` holds** — the common case, where
/// a render has an account id in hand and not an instrument. An unknown account
/// renders the bare minor value.
#[must_use]
pub fn render_amount_on(snapshot: &EconomySnapshot, account: AccountId, amount: Amount) -> String {
    match snapshot.account(account) {
        Some(a) => render_amount(snapshot, &a.instrument, amount),
        None => amount.to_decimal_string(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{
        InstrumentDef, InstrumentDisplay, InstrumentKind, InstrumentStatus, NegativePolicy,
    };
    use smol_str::SmolStr;

    fn instrument(code: &str, scale: u8, display: Option<InstrumentDisplay>) -> InstrumentDef {
        InstrumentDef {
            id: InstrumentId::new(code).expect("code"),
            ruleset: SmolStr::new_static("test"),
            code: SmolStr::new(code),
            kind: InstrumentKind::Currency,
            scale,
            transferable: true,
            negative_policy: NegativePolicy::SystemOnly,
            status: InstrumentStatus::Active,
            display,
            revision: 1,
        }
    }

    #[test]
    fn e6s1_s5_prefix_suffix_scale_and_sign_render_through_one_function() {
        let mut s = EconomySnapshot::new();
        s.put_instrument(instrument(
            "nuyen",
            0,
            InstrumentDisplay::new("¥", SymbolPosition::Prefix),
        ));
        s.put_instrument(instrument(
            "euro",
            2,
            InstrumentDisplay::new("€", SymbolPosition::Suffix),
        ));
        s.put_instrument(instrument("plain", 2, None));
        let nuyen = InstrumentId::new("nuyen").expect("id");
        let euro = InstrumentId::new("euro").expect("id");
        let plain = InstrumentId::new("plain").expect("id");
        assert_eq!(
            render_amount(&s, &nuyen, Amount::from_minor(1_000)),
            "¥1000"
        );
        assert_eq!(render_amount(&s, &nuyen, Amount::from_minor(-5)), "-¥5");
        assert_eq!(
            render_amount(&s, &euro, Amount::from_minor(1_250)),
            "12.50€"
        );
        assert_eq!(render_amount(&s, &euro, Amount::from_minor(-5)), "-0.05€");
        assert_eq!(
            render_amount(&s, &plain, Amount::from_minor(1_250)),
            "12.50"
        );
        // Unknown instrument: the bare minor value, never a refusal.
        let unknown = InstrumentId::new("nope").expect("id");
        assert_eq!(render_amount(&s, &unknown, Amount::from_minor(7)), "7");
        // The symbol is bounded and whitespace-free.
        assert!(InstrumentDisplay::new("", SymbolPosition::Prefix).is_none());
        assert!(InstrumentDisplay::new("¥ ", SymbolPosition::Prefix).is_none());
        assert!(InstrumentDisplay::new("¥¥¥¥¥¥¥¥¥", SymbolPosition::Prefix).is_none());
        assert!(
            serde_json::from_str::<InstrumentDisplay>(
                r#"{"symbol":"too long a symbol","position":"Prefix"}"#
            )
            .is_err()
        );
    }
}
