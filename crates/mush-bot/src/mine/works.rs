//! **The first production story's lines** (doc 39 cut IN.5; §6's IN.5 row): the player's half of
//! the trade-and-works run as the bot types it over SSH — the passage between Ceres and Stickney
//! Yards, the Exchange's and Kasei's asks, Kasei's and the Co-op's bids, and the works' order
//! contract (`+works`) — and the reader for the server's answers. Written from `help trade`,
//! `help ships` and `+works`'s own help, as the rest of the lane is from the player help; the
//! `#[cfg(test)]` module checks each sentence against `tacsim`'s `pub` constants through the
//! development dependency, the `lines` module's precedent.
//!
//! Taken here: `+works`, `+works/quote|order <recipe> [qty]`, `+works/deliver <order>=<good>
//! <qty>`, `+works/collect <order>`, `+works/cancel <order>`, and `+ship/plan|undock <port>` for
//! the ports of the run ([`PORTS`]). Already taken and not repeated: `+market/list`, `+market/quote
//! <n>=<qty>`, `+market/accept <n>`, `+market/sell <n>=all`, `+ship/board` and `+ship/leave`
//! (the mining lane's, [`super::in_vocabulary`]).
//!
//! No lane types these lines yet: the story drives them from the test (`mining_cohort`'s `in5_`
//! stories), after the mining lane has sold its hold.

use crate::mine::lines::{self, Listing};

/// The works verb.
pub const WORKS: &str = "+works";
/// The yard verb (doc 39 cut IN.11): `+works`' grammar at a yard.
pub const YARD: &str = "+yard";
/// `tacsim::port::works::BOOKED_AT` at the yard: "Order {n} booked at the yard: …".
pub const YARD_BOOKED: &str = " booked at the yard: ";
/// `tacsim::port::works::COMMISSIONS_AS`: "Order {n}: the yard commissions her as {mark}; …".
pub const COMMISSIONS_AS: &str = ": the yard commissions her as ";

/// The ports the production run sails between, by the route word `+ship/plan` and `+ship/undock`
/// take: Ceres and Stickney Yards, and the way out by Luna and Deimos (Aitken Highport, Voltaire
/// Anchorage), whose legs no picket interdicts.
pub const PORTS: [&str; 4] = ["ceres", "deimos", "luna", "stickney"];

/// `tacsim::port::works::BOOKED_AT` at the works: "Order {n} booked at the works: …".
pub const BOOKED: &str = " booked at the works: ";
/// `tacsim::port::works::DELIVERED_AT` at the works: "Order {n} is delivered at the works: …".
pub const DELIVERED: &str = " is delivered at the works: ";
/// `tacsim::port::works::INTO_ESCROW`: "{units} {good} moves into order {n}'s escrow.".
pub const INTO_ESCROW: &str = "'s escrow.";
/// `tacsim::port::works::INTO_HOLD`: "Order {n}: {goods} move into your hold.".
pub const INTO_HOLD: &str = " move into your hold.";
/// `tacsim::port::works::FEE_CLAUSE`: the quote's fee, "…; fee {cr}; booking fee {cr}.".
pub const FEE: &str = "; fee ";
/// `tacsim::port::works::SETTLING`: a collection asked for before the arm has pooled the output.
pub const SETTLING: &str = "That order is still settling.";

/// **Whether `line` is one of the production run's lines**, each in closed form:
///
/// - `+works`, `+works/quote <recipe> [qty]`, `+works/order <recipe> [qty]`, `+works/deliver
///   <order>=<good> <qty>`, `+works/collect <order>`, `+works/cancel <order>` — `<recipe>` and
///   `<good>` a lower-case word of letters, digits, `_` and `.`, `<qty>` 1–999, `<order>` a
///   decimal number;
/// - `+ship/plan <port>` and `+ship/undock <port>`, `<port>` one of [`PORTS`].
#[must_use]
pub fn in_vocabulary(line: &str) -> bool {
    if line.len() > 120 || line.contains(['\n', '\r']) {
        return false;
    }
    if line == WORKS || line == YARD {
        return true;
    }
    if let Some(rest) = line
        .strip_prefix("+works/")
        .or_else(|| line.strip_prefix("+yard/"))
    {
        return works_switch(rest);
    }
    for verb in ["+ship/plan ", "+ship/undock "] {
        if let Some(port) = line.strip_prefix(verb) {
            return PORTS.contains(&port);
        }
    }
    matter_line(line)
}

/// **Doc 39 cut IN.11's lines**: a part lot sold into a bid (`+market/sell <n>=1`), Stickney's
/// walk to the Service Counter (`customs`, `service`), and the bay's look (by dbref or yard mark), replace quote, stow, berth and fit of a hull named by its dbref (`#<n>`), the
/// stow's carrier by its name, the fit's part by its good.
fn matter_line(line: &str) -> bool {
    let dbref = |s: &str| s.strip_prefix('#').is_some_and(|n| number(n, 9));
    if let Some(rest) = line.strip_prefix("+market/sell ") {
        return rest.strip_suffix("=1").is_some_and(|n| number(n, 20));
    }
    // Stickney's walk from the berth gallery to the Service Counter (lobby, customs,
    // concourse, service) and back.
    if matches!(line, "customs" | "service") {
        return true;
    }
    // A yard mark (`SYB-01`): three capitals, a hyphen, two digits.
    let mark = |s: &str| {
        s.len() == 6
            && s[..3].bytes().all(|b| b.is_ascii_uppercase())
            && s.as_bytes()[3] == b'-'
            && s[4..].bytes().all(|b| b.is_ascii_digit())
    };
    if let Some(rest) = line.strip_prefix("+bay ") {
        return dbref(rest) || mark(rest);
    }
    if let Some(rest) = line.strip_prefix("+bay/berth ") {
        return dbref(rest);
    }
    if let Some(rest) = line.strip_prefix("+bay/quote ") {
        return rest.strip_suffix("=replace").is_some_and(dbref);
    }
    if let Some((hull, carrier)) = line
        .strip_prefix("+bay/stow ")
        .and_then(|r| r.split_once('='))
    {
        return dbref(hull)
            && (1..=32).contains(&carrier.len())
            && carrier.starts_with(|c: char| c.is_ascii_alphabetic())
            && carrier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '\'' | '.'));
    }
    if let Some((hull, fit)) = line
        .strip_prefix("+bay/fit ")
        .and_then(|r| r.split_once('='))
    {
        return dbref(hull)
            && fit
                .split_once(' ')
                .is_some_and(|(slot, good)| word(slot) && word(good));
    }
    false
}

/// **A hull's dbref as `+bay` prints it** ("{name} (#{n}) stands in …"): the number after
/// `name`'s first " (#".
#[must_use]
pub fn bay_ref(window: &str, name: &str) -> Option<u32> {
    let window = lines::normalise(window);
    let lead = format!("{name} (#");
    let at = window.find(&lead)?;
    let rest = &window[at + lead.len()..];
    rest.split(')').next()?.parse().ok()
}

/// A decimal number of up to `len` digits with no leading zero.
fn number(s: &str, len: usize) -> bool {
    (1..=len).contains(&s.len()) && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit())
}

/// A recipe or good word: 1–64 lower-case letters, digits, `_` and `.`, starting with a letter.
fn word(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.'))
}

fn works_switch(rest: &str) -> bool {
    let Some((switch, args)) = rest.split_once(' ') else {
        return false;
    };
    match switch {
        "quote" | "order" => match args.split_once(' ') {
            Some((recipe, qty)) => word(recipe) && number(qty, 3),
            None => word(args),
        },
        "deliver" => args.split_once('=').is_some_and(|(order, what)| {
            number(order, 20)
                && what
                    .split_once(' ')
                    .is_some_and(|(good, qty)| word(good) && number(qty, 3))
        }),
        "collect" | "cancel" => number(args, 20),
        _ => false,
    }
}

/// **The order a booking names** ("Order {n} booked at the works: …"): the number before the
/// first [`BOOKED`] in `window` (normalised).
#[must_use]
pub fn booked(window: &str) -> Option<u64> {
    let window = lines::normalise(window);
    let at = window.find(BOOKED)?;
    let head = window[..at].rsplit("Order ").next()?;
    head.parse().ok()
}

/// **The order a yard booking names** ("Order {n} booked at the yard: …").
#[must_use]
pub fn yard_booked(window: &str) -> Option<u64> {
    let window = lines::normalise(window);
    let at = window.find(YARD_BOOKED)?;
    let head = window[..at].rsplit("Order ").next()?;
    head.parse().ok()
}

/// **The mark the yard commissions `order` under** ("Order {n}: the yard commissions her as
/// {mark}; …").
#[must_use]
pub fn commissioned(window: &str, order: u64) -> Option<String> {
    let window = lines::normalise(window);
    let lead = format!("Order {order}{COMMISSIONS_AS}");
    let at = window.find(&lead)?;
    let rest = &window[at + lead.len()..];
    Some(rest.split(';').next()?.trim().to_owned())
}

/// Whether `window` tells that `order` is delivered at the works.
#[must_use]
pub fn delivered(window: &str, order: u64) -> bool {
    lines::has(
        &lines::normalise(window),
        &format!("Order {order}{DELIVERED}"),
    )
}

/// Whether `window` holds a delivery of the bill into `order`'s escrow.
#[must_use]
pub fn supplied(window: &str, order: u64) -> bool {
    lines::has(
        &lines::normalise(window),
        &format!(" moves into order {order}{INTO_ESCROW}"),
    )
}

/// Whether `window` holds `order`'s collection into the hold.
#[must_use]
pub fn collected(window: &str, order: u64) -> bool {
    let window = lines::normalise(window);
    window
        .match_indices(&format!("Order {order}: "))
        .any(|(at, _)| {
            window[at..]
                .split_once(INTO_HOLD)
                .is_some_and(|(sentence, _)| !sentence.contains(". "))
        })
}

/// **The quote's fee** ("…; fee {cr}; booking fee …"), as printed.
#[must_use]
pub fn quote_fee(window: &str) -> Option<String> {
    let window = lines::normalise(window);
    let at = window.find(FEE)?;
    let rest = &window[at + FEE.len()..];
    Some(rest.split(';').next()?.trim().to_owned())
}

/// The quantity a `+market/list` row offers or wants: the number after the first " — " (an ask,
/// "#{n} {name} — {qty} × {price} …") or after " — wanted " (a bid).
#[must_use]
pub fn row_quantity(row: &Listing) -> Option<u64> {
    let after = row
        .text
        .split_once(" — wanted ")
        .or_else(|| row.text.split_once(" — "))?
        .1;
    after.split(' ').next()?.replace(',', "").parse().ok()
}

/// The ask whose row holds `label` (case-insensitive) and is not `wanted`, with its quantity.
#[must_use]
pub fn ask_row<'r>(rows: &'r [Listing], label: &str) -> Option<&'r Listing> {
    let label = label.to_lowercase();
    rows.iter()
        .find(|row| !row.wanted && row.text.to_lowercase().contains(&label))
}

/// The buying bid whose row holds `label` (case-insensitive).
#[must_use]
pub fn bid_row<'r>(rows: &'r [Listing], label: &str) -> Option<&'r Listing> {
    let label = label.to_lowercase();
    rows.iter()
        .find(|row| row.wanted && !row.not_buying && row.text.to_lowercase().contains(&label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in5_the_production_lines_are_closed() {
        for line in [
            "+works",
            "+works/quote fittings",
            "+works/quote fittings 2",
            "+works/order fittings",
            "+works/deliver 154=structural_alloys 5",
            "+works/deliver 154=carbon_ore 1",
            "+works/collect 154",
            "+works/cancel 154",
            "+yard",
            "+yard/quote dray",
            "+yard/order tv_escort",
            "+yard/deliver 161=tacsim.part.plant.hauler_mk1 1",
            "+yard/collect 161",
            "+market/sell 31=1",
            "+bay #813",
            "+bay SYB-01",
            "customs",
            "service",
            "+bay/quote #813=replace",
            "+bay/stow #813=Long Ox",
            "+bay/berth #813",
            "+bay/fit #813=power tacsim.part.plant.light_mk1",
            "+ship/plan stickney",
            "+ship/undock stickney",
            "+ship/undock ceres",
            "+ship/undock luna",
            "+ship/plan deimos",
        ] {
            assert!(in_vocabulary(line), "{line}");
        }
        for line in [
            "+works/quote",
            "+works/quote Fittings",
            "+works/quote fittings 0",
            "+works/order fittings 1000",
            "+works/deliver 154=structural_alloys",
            "+works/deliver 0154=carbon_ore 1",
            "+works/collect",
            "+works/collect x",
            "+works/strip 154",
            "+ship/undock titan",
            "+ship/undock",
            "@works/order fittings",
        ] {
            assert!(!in_vocabulary(line), "{line}");
        }
    }

    /// The reader, held to tacsim's own sentences (the `lines` module's precedent).
    #[test]
    fn in5_the_reader_reads_the_works_sentences() {
        use tacsim::port::works::{
            BOOKED_AT, DELIVERED_AT, FEE_CLAUSE, INTO_ESCROW as GAME_ESCROW,
            INTO_HOLD as GAME_HOLD, SETTLING as GAME_SETTLING,
        };
        assert_eq!(BOOKED, format!("{BOOKED_AT}works: "));
        assert_eq!(DELIVERED, format!("{DELIVERED_AT}works: "));
        assert_eq!(INTO_ESCROW, GAME_ESCROW);
        assert_eq!(INTO_HOLD, GAME_HOLD);
        assert_eq!(FEE, FEE_CLAUSE);
        assert_eq!(SETTLING, GAME_SETTLING);
        assert_eq!(YARD_BOOKED, format!("{BOOKED_AT}yard: "));
        assert_eq!(COMMISSIONS_AS, tacsim::port::works::COMMISSIONS_AS);
        assert_eq!(
            yard_booked("Order 161 booked at the yard: dray x1, 12 IC days. Deliver the bill: …"),
            Some(161)
        );
        assert_eq!(
            commissioned(
                "Order 161: the yard commissions her as SYB-01; she is handed over when the \n                 work is done.",
                161
            ),
            Some("SYB-01".to_owned())
        );
        assert_eq!(
            bay_ref("SYB-01 (#813) stands in a workshop.", "SYB-01"),
            Some(813)
        );
        let book = "Order 154 booked at the works: fittings x1, 1 IC days. Deliver the bill: 5 \
                    structural_alloys, 1 carbon_ore. Booking fee 847.00cr held (forfeit on a \
                    cancel).";
        assert_eq!(booked(book), Some(154));
        assert_eq!(booked("Order 154 is delivered at the works"), None);
        assert!(delivered(
            "Order 154 is delivered at the works: `+works /collect 154` takes it.",
            154
        ));
        assert!(!delivered("Order 1540 is delivered at the works: ", 154));
        assert!(supplied(
            "5 structural_alloys moves into order 154's escrow.",
            154
        ));
        assert!(collected(
            "Order 154: 5 industrial_parts move into your hold.",
            154
        ));
        assert!(!collected(
            "Order 154 is closed; its escrow was empty.",
            154
        ));
        assert_eq!(
            quote_fee(
                "fittings x1: bill 5 structural_alloys, 1 carbon_ore; 1 IC days (6 work units); \
                 fee 265.00cr; booking fee 847.00cr."
            ),
            Some("265.00cr".to_owned())
        );
        let rows = lines::listings(&[
            "#12 structural alloy stock — 7,016 × 450.00cr [stickney] at Kasei Metals".to_owned(),
            "#13 metal concentrate — wanted 3,980 × 280.00cr [stickney] by Kasei Metals".to_owned(),
            "#14 yard fittings, crated — wanted 5,400 × 600.00cr [ceres] by Belt Carriers' \
             Cooperative — not buying"
                .to_owned(),
        ]);
        let ask = ask_row(&rows, "structural alloy").expect("the alloy ask");
        assert_eq!((ask.number, row_quantity(ask)), (12, Some(7_016)));
        let bid = bid_row(&rows, "metal concentrate").expect("Kasei's bid");
        assert_eq!((bid.number, row_quantity(bid)), (13, Some(3_980)));
        assert!(bid_row(&rows, "yard fittings").is_none(), "not buying");
    }
}
