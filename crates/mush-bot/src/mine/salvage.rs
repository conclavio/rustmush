//! **The salvage loop's lines** (doc 41 cut RS.0; Q-38-29): the player's half of doc 41's stories
//! (a)–(e) as the bot types it over SSH, and the reader for the server's answers. Written from
//! `help field`'s recovery section, `+salvage`'s and `+bay`'s own help and doc 41's ruled forms,
//! as the rest of the lane is from the player help; the `#[cfg(test)]` module checks each
//! sentence against `tacsim`'s `pub` constants through the development dependency, the `lines`
//! module's precedent.
//!
//! Taken here: `+tv/<n> pickup|scrap`, every `+salvage` switch and every `+bay` switch. Already
//! taken before this cut and not repeated: `+field/void` (doc 40 cut FR.0,
//! [`super::field::round_word`]), `+sortie/condition` (doc 42 cut CR.0, [`super::crew`]), `+tv/<n>
//! tow|load|release` (doc 36 cut GC.4 and FR.0) and the four-digit address
//! ([`crate::mine::lines::is_addr`], FR.0).
//!
//! The field lane ([`super::field::in_vocabulary`]) and the escort lane
//! ([`super::escort::in_vocabulary`]) both accept [`in_vocabulary`]; no lane types any of these
//! lines yet (the stories grow them from cut RS.1 on).

/// The salvage verb.
pub const SALVAGE: &str = "+salvage";
/// The bay verb.
pub const BAY: &str = "+bay";

/// The contest kinds (`ContestKind`, Q-41-23).
const CONTEST_KINDS: [&str; 3] = ["recovery", "procedure", "right"];
/// The bay's work kinds (`+bay/order <hull>=<work>`).
const WORK_KINDS: [&str; 7] = [
    "all", "armor", "rebuild", "mend", "unjam", "replace", "rearm",
];

/// **Whether `line` is one of the salvage loop's lines**, each in closed form:
///
/// - `+tv/<n> pickup [<object>] [go]` and `+tv/<n> scrap [<object>] [go]` (doc 36 cuts GC.5 and
///   GC.6), `<n>` 1–99, `<object>` a decimal object number;
/// - `+salvage`, `+salvage/claim <hull> [go]`, `+salvage/contest <hull>[=recovery|procedure|right]
///   [go]`, `+salvage/inspect <hull>`, `+salvage/report <hull>=<who> [go]`, `+salvage/authorize
///   <hull>=<who> [until=<n>] [go]` and `+salvage/authorize <site>/<block>=<who> [until=<n>] [go]`
///   (as shipped by doc 36 and RS.3); RS.5's `+salvage/retrieve <hull>[=<port>] [fee=<cr>] [go]`,
///   `+salvage/jobs` and `+salvage/accept <job> [go]` (Q-41-9); RS.7's `+salvage/report
///   <site>[=<who>|#<hull>] registry` (Q-41-24);
/// - `+bay <hull>`, `+bay/quote <hull>`, `+bay/order <hull>=<work>[,<work>…]`, `+bay/fit
///   <hull>=<slot> <lot>`, `+bay/strip <hull>=<part>`, `+bay/scrap|cancel|berth <hull>`, and
///   `+bay/stow <hull>[=<carrier>]` (RS.4's bay-to-bay form, doc 39's IN.9 verb).
///
/// A `<hull>` is `#<id>` or a name; a `<who>` a plain name or `#<id>`; a site a route word.
#[must_use]
pub fn in_vocabulary(line: &str) -> bool {
    if line.len() > 160 || line.contains(['\n', '\r']) {
        return false;
    }
    if let Some(rest) = line.strip_prefix("+tv/") {
        return tv_word(rest);
    }
    if line == SALVAGE {
        return true;
    }
    if let Some(rest) = line.strip_prefix("+salvage/") {
        return salvage_switch(rest);
    }
    if let Some(rest) = line.strip_prefix("+bay") {
        return bay_switch(rest);
    }
    false
}

/// A decimal number of up to `len` digits with no leading zero.
fn number(s: &str, len: usize) -> bool {
    (1..=len).contains(&s.len()) && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit())
}

/// A hull as `+bay` and `+salvage` name one: `#<id>`, or a name of letters, digits, spaces,
/// hyphens, apostrophes and full stops, 1–48, starting with a letter or digit and untrimmed of
/// nothing.
fn hull(s: &str) -> bool {
    if let Some(id) = s.strip_prefix('#') {
        return number(id, 10);
    }
    (1..=48).contains(&s.len())
        && s.trim() == s
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '\'' | '.'))
}

/// A character: `#<id>` or a plain name of 1–32 letters and digits.
fn who(s: &str) -> bool {
    match s.strip_prefix('#') {
        Some(id) => number(id, 10),
        None => (1..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric()),
    }
}

/// A lowercase word of 1–32 letters, digits and underscores: a site's route word, a port's, a
/// slot's, a section's or a part's.
fn word(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// `rest` without a trailing ` go`, and whether it had one.
fn split_go(rest: &str) -> (&str, bool) {
    rest.strip_suffix(" go")
        .map_or((rest, false), |head| (head, true))
}

/// `<n> pickup|scrap [<object>] [go]` after `+tv/`.
fn tv_word(rest: &str) -> bool {
    let Some((rig, tail)) = rest.split_once(' ') else {
        return false;
    };
    if !number(rig, 2) {
        return false;
    }
    let (tail, _) = split_go(tail);
    let Some(after) = tail
        .strip_prefix("pickup")
        .or_else(|| tail.strip_prefix("scrap"))
    else {
        return false;
    };
    after.is_empty() || after.strip_prefix(' ').is_some_and(|n| number(n, 10))
}

/// Everything after `+salvage/`.
fn salvage_switch(rest: &str) -> bool {
    if rest == "jobs" {
        return true;
    }
    let Some((switch, args)) = rest.split_once(' ') else {
        return false;
    };
    let (body, go) = split_go(args);
    match switch {
        "claim" => hull(body),
        "inspect" => !go && hull(args),
        "contest" => match body.split_once('=') {
            Some((h, kind)) => hull(h) && CONTEST_KINDS.contains(&kind),
            None => hull(body),
        },
        "report" => {
            // RS.7's registry report: `<site>[=<who>|#<hull>] registry`, never with `go`.
            if let Some(head) = args.strip_suffix(" registry") {
                return match head.split_once('=') {
                    Some((site, target)) => word(site) && who(target),
                    None => word(head),
                };
            }
            body.split_once('=').is_some_and(|(h, w)| hull(h) && who(w))
        }
        "authorize" => {
            let Some((what, grantee)) = body.split_once('=') else {
                return false;
            };
            let grantee = match grantee.split_once(' ') {
                Some((w, until)) => {
                    if !until.strip_prefix("until=").is_some_and(|n| number(n, 12)) {
                        return false;
                    }
                    w
                }
                None => grantee,
            };
            let block = |s: &str| {
                s.split_once('/').is_some_and(|(site, letter)| {
                    word(site)
                        && letter.len() == 1
                        && ('A'..='P').contains(&letter.chars().next().unwrap_or('?'))
                })
            };
            who(grantee) && (block(what) || hull(what))
        }
        "retrieve" => {
            // RS.5 (Q-41-9): `<hull>[=<port>] [fee=<cr>] [go]`.
            let (head, fee) = match body.rsplit_once(" fee=") {
                Some((head, fee)) => (head, Some(fee)),
                None => (body, None),
            };
            fee.is_none_or(|fee| number(fee, 9))
                && match head.split_once('=') {
                    Some((h, port)) => hull(h) && word(port),
                    None => hull(head),
                }
        }
        "accept" => number(body.strip_prefix('#').unwrap_or(body), 10),
        _ => false,
    }
}

/// Everything after `+bay`.
fn bay_switch(rest: &str) -> bool {
    if let Some(h) = rest.strip_prefix(' ') {
        return hull(h);
    }
    let Some((switch, args)) = rest.strip_prefix('/').and_then(|r| r.split_once(' ')) else {
        return false;
    };
    match switch {
        "quote" | "scrap" | "cancel" | "berth" => hull(args),
        "stow" => match args.split_once('=') {
            Some((h, carrier)) => hull(h) && hull(carrier),
            None => hull(args),
        },
        "order" => args.split_once('=').is_some_and(|(h, work)| {
            hull(h)
                && work.split(',').count() <= 8
                && work.split(',').all(|w| WORK_KINDS.contains(&w) || word(w))
        }),
        "strip" => args
            .split_once('=')
            .is_some_and(|(h, part)| hull(h) && word(part)),
        "fit" => args.split_once('=').is_some_and(|(h, fit)| {
            hull(h)
                && fit.split_once(' ').is_some_and(|(slot, lot)| {
                    let hex = lot.strip_prefix("lot#").unwrap_or(lot);
                    word(slot) && hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit())
                })
        }),
        _ => false,
    }
}

/// **A salvage answer the server said**: the claim's, the contest's and the rig's scrap and
/// pickup, each by the game's own sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// No custody record names the hull (`NOT_IN_CUSTODY`).
    NotInCustody,
    /// The claim previewed (`CLAIM_PREVIEW`).
    ClaimPreviewed,
    /// The claim filed (`CLAIM_FILED`).
    ClaimFiled,
    /// The claim was filed already (`ALREADY_FILED`).
    AlreadyFiled,
    /// A contest previewed, on any ground (`CONTEST_PREVIEW_*`).
    ContestPreviewed,
    /// The contest filed (`CONTEST_FILED`).
    ContestFiled,
    /// A contest refused for want of a ground (`NO_GROUND`, `NO_RECOVERY_GROUND`, `NO_DEFECT`,
    /// `NO_RIGHT`, `NOT_ANSWERING`).
    NoGround,
    /// The contest window has closed (`WINDOW_CLOSED`).
    WindowClosed,
    /// The rig's scrap previewed (`SCRAP_PREVIEW`, `SCRAP_LOSS_PREVIEW`).
    ScrapPreviewed,
    /// The remains broken up (`SCRAPPED`, `SCRAPPED_LOSS`).
    Scrapped,
    /// No remains in reach (`NO_REMAINS`, `REMAINS_NOT_IN_REACH`).
    NoRemains,
    /// Only the title holder breaks them up (`SCRAP_NOT_TITLED`).
    ScrapNotTitled,
    /// The rig's pickup previewed or queued (`PICKUP_PREVIEW`, `PICKED_UP`).
    PickedUp,
    /// No crate in reach (`NO_CRATE`, `CRATE_NOT_IN_REACH`).
    NoCrate,
}

/// The claim's and the contest's answers by their fixed head and tail: `(head, tail, answer)`.
const ANSWERS: [(&str, &str, Answer); 16] = [
    ("No salvage custody names ", ".", Answer::NotInCustody),
    (
        "You would file a salvage claim on the ",
        " go",
        Answer::ClaimPreviewed,
    ),
    ("You file a salvage claim on the ", ".", Answer::ClaimFiled),
    ("The claim on the ", ").", Answer::AlreadyFiled),
    (
        "You would contest the claim on the ",
        " go",
        Answer::ContestPreviewed,
    ),
    (
        "You file your contest of the claim on the ",
        ".",
        Answer::ContestFiled,
    ),
    (
        "You hold neither evidence of how the ",
        " nor its title.",
        Answer::NoGround,
    ),
    (
        "You hold no record of how the ",
        " was recovered.",
        Answer::NoGround,
    ),
    (
        "The registry's own records show no defect",
        ".",
        Answer::NoGround,
    ),
    (
        "You hold no right that bound ",
        " is no ground against their claim.",
        Answer::NoGround,
    ),
    (
        "Only someone who answers for the ",
        "'s title may contest the claim's procedure.",
        Answer::NoGround,
    ),
    (
        "The contest window on the ",
        " has closed.",
        Answer::WindowClosed,
    ),
    (
        "No remains lie within a cell of rig ",
        ".",
        Answer::NoRemains,
    ),
    (
        "Only the title holder of ",
        " once the registry awards it.",
        Answer::ScrapNotTitled,
    ),
    ("No crate lies within a cell of rig ", ".", Answer::NoCrate),
    (
        "Object ",
        " is not a crate within a cell of rig ",
        Answer::NoCrate,
    ),
];

/// **Read one line of the server's text as a salvage answer**, if it is one.
#[must_use]
pub fn answer(text: &str) -> Option<Answer> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("Rig ") {
        // The rig's own sentences: the break-up's and the pickup's verbs tell them apart.
        if rest.contains(" can break up ") && rest.contains(" where it lies: ") {
            return rest
                .ends_with(" scrap go")
                .then_some(Answer::ScrapPreviewed);
        }
        if rest.contains(" breaks up ") && rest.contains(" where it lies: ") {
            return rest.ends_with("'s hold.").then_some(Answer::Scrapped);
        }
        if rest.contains(" aboard into its hopper") {
            return (rest.ends_with(" pickup go") || rest.ends_with(": queued."))
                .then_some(Answer::PickedUp);
        }
        return None;
    }
    if let Some(rest) = text.strip_prefix("Object ")
        && rest.contains(" is not remains within a cell of rig ")
    {
        return Some(Answer::NoRemains);
    }
    ANSWERS.iter().find_map(|&(head, tail, answer)| {
        let rest = text.strip_prefix(head)?;
        let hit = if tail.ends_with(' ') {
            rest.contains(tail)
        } else {
            rest.ends_with(tail)
        };
        hit.then_some(answer)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rs0_the_salvage_lines_are_closed() {
        for line in [
            "+tv/1 pickup",
            "+tv/1 pickup go",
            "+tv/2 pickup 4417 go",
            "+tv/1 scrap",
            "+tv/1 scrap go",
            "+tv/12 scrap 90210",
            "+salvage",
            "+salvage/claim #812",
            "+salvage/claim Salt Moth go",
            "+salvage/contest #812",
            "+salvage/contest #812 go",
            "+salvage/contest #812=recovery go",
            "+salvage/contest #812=procedure",
            "+salvage/contest Rig 4=right go",
            "+salvage/inspect #812",
            "+salvage/report #812=Tamsin go",
            "+salvage/report urvara=#77 registry",
            "+salvage/report ahuna registry",
            "+salvage/authorize #812=Orin go",
            "+salvage/authorize #812=Orin until=1790000000 go",
            "+salvage/authorize urvara/G=Orin go",
            "+salvage/retrieve #812",
            "+salvage/retrieve #812=ceres fee=4000 go",
            "+salvage/jobs",
            "+salvage/accept 3 go",
            "+salvage/accept #3",
            "+bay #812",
            "+bay/quote Salt Moth",
            "+bay/order #812=all",
            "+bay/order #812=mend,unjam,replace",
            "+bay/order #812=left_arm",
            "+bay/fit #812=drive lot#00ff00ff00ff00ff",
            "+bay/strip #812=autocannon",
            "+bay/scrap #812",
            "+bay/cancel #812",
            "+bay/berth #812",
            "+bay/stow #812",
            "+bay/stow #812=Salt Moth",
        ] {
            assert!(in_vocabulary(line), "{line}");
        }
        for line in [
            "+tv/0 scrap",
            "+tv/100 scrap",
            "+tv/1 scrap 007",
            "+tv/1 scrap go go",
            "+tv/1 scrap go 4",
            "+tv/1 pickup all",
            "+tv/1 release",
            "+salvage/claim",
            "+salvage/claim #",
            "+salvage/claim #81x",
            "+salvage/claim  Salt Moth",
            "+salvage/contest #812=theft",
            "+salvage/inspect #812 go",
            "+salvage/authorize #812",
            "+salvage/authorize #812=Orin until=soon",
            "+salvage/authorize urvara/Q=Orin",
            "+salvage/retrieve #812=Ceres Freeport",
            "+salvage/accept",
            "+salvage/forsale",
            "+salvage/claim #812\nsay hi",
            "+bay",
            "+bay/order #812",
            "+bay/order #812=Mend",
            "+bay/fit #812=drive lot#00ff",
            "+bay/return #812",
            "@salvage/claim #812",
        ] {
            assert!(!in_vocabulary(line), "{line}");
        }
    }

    /// The contract check: each reader matches the game's own sentence.
    #[test]
    fn rs0_the_salvage_answers_match_the_game() {
        use tacsim::surface::{claim, salvage};
        let fill = |s: &str| {
            s.replace("{hull}", "#812")
                .replace("{name}", "Salt Moth")
                .replace("{registry}", "Ceres Registry")
                .replace("{window}", "2 IC days")
                .replace("{told}", "Tamsin is told.")
                .replace("{contract}", "41")
                .replace("{kind}", "a pickup")
                .replace("{defect}", "a missed answerer")
                .replace("{claimant}", "Orin")
                .replace("{right}", "grant/7")
                .replace("{n}", "1")
                .replace("{id}", "4417")
                .replace("{what}", "the remains of #812")
                .replace("{t}", "40")
                .replace("{carrier}", "Dray-P Kestrel")
        };
        for (line, want) in [
            (claim::NOT_IN_CUSTODY, Answer::NotInCustody),
            (claim::CLAIM_PREVIEW, Answer::ClaimPreviewed),
            (claim::CLAIM_FILED, Answer::ClaimFiled),
            (claim::ALREADY_FILED, Answer::AlreadyFiled),
            (claim::CONTEST_PREVIEW_EVIDENCE, Answer::ContestPreviewed),
            (claim::CONTEST_PREVIEW_TITLE, Answer::ContestPreviewed),
            (claim::CONTEST_PREVIEW_PROCEDURE, Answer::ContestPreviewed),
            (claim::CONTEST_PREVIEW_RIGHT, Answer::ContestPreviewed),
            (claim::CONTEST_FILED, Answer::ContestFiled),
            (claim::NO_GROUND, Answer::NoGround),
            (claim::NO_RECOVERY_GROUND, Answer::NoGround),
            (claim::NO_DEFECT, Answer::NoGround),
            (claim::NO_RIGHT, Answer::NoGround),
            (claim::NOT_ANSWERING, Answer::NoGround),
            (claim::WINDOW_CLOSED, Answer::WindowClosed),
            (claim::SCRAP_PREVIEW, Answer::ScrapPreviewed),
            (claim::SCRAP_LOSS_PREVIEW, Answer::ScrapPreviewed),
            (claim::SCRAPPED, Answer::Scrapped),
            (claim::SCRAPPED_LOSS, Answer::Scrapped),
            (claim::NO_REMAINS, Answer::NoRemains),
            (claim::REMAINS_NOT_IN_REACH, Answer::NoRemains),
            (claim::SCRAP_NOT_TITLED, Answer::ScrapNotTitled),
            (salvage::PICKUP_PREVIEW, Answer::PickedUp),
            (salvage::PICKED_UP, Answer::PickedUp),
            (salvage::NO_CRATE, Answer::NoCrate),
            (salvage::CRATE_NOT_IN_REACH, Answer::NoCrate),
        ] {
            let said = fill(line);
            assert_eq!(answer(&said), Some(want), "{said}");
        }
        for other in [
            claim::NO_CLAIMS,
            claim::WINDOW_PAUSED,
            claim::SALVAGE_USAGE,
            "Rig 1 arrives at G1000.",
        ] {
            assert_eq!(answer(&fill(other)), None, "{other}");
        }
    }
}
