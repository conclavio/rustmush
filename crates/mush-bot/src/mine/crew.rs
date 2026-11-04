//! **The crew's lines** (doc 42 cut CR.0; Q-42-23, doc 38 Q-38-29): the captive's half and the
//! captor's half a story types over SSH, and the readers for the custody lines the server says
//! back. Written from `help field`'s Crew section, as the rest of the lane is from the player
//! help; the `#[cfg(test)]` module checks each sentence against `tacsim`'s `pub` constants
//! through the development dependency, the `lines` module's precedent.
//!
//! The field lane ([`super::field::in_vocabulary`]) and the escort lane
//! ([`super::escort::in_vocabulary`]) both accept [`in_vocabulary`]; no lane types any of these
//! lines yet (the stories grow them from cut CR.1 on).

/// The captive's status read.
pub const CONDITION: &str = "+sortie/condition";
/// The ejection (doc 42 Q-42-10; cut CR.4 builds the verb).
pub const EJECT: &str = "+sortie/eject";
/// The extraction request, refused while held and allowed after the bound (Q-42-8).
pub const EXTRACT: &str = "+sortie/extract";
/// The captor's list (doc 42 Q-42-9; cut CR.1 builds the verb).
pub const CAPTIVES: &str = "+ship/captives";
/// The captor's release at will: `+ship/release <who>` (Q-42-9).
pub const RELEASE: &str = "+ship/release ";

/// The held refusal's head ("You are held aboard the {carrier}.").
const HELD_HEAD: &str = "You are held aboard the ";
/// The pickup refusal.
const PICKUP: &str = "You are waiting for your pickup.";
/// The processing refusal's head ("You are being processed at {port}; about {h} IC h.").
const PROCESSING_HEAD: &str = "You are being processed at ";

/// **Whether `line` is one of the crew's lines**, each in closed form: the three `+sortie` crew
/// verbs, `+ship/captives`, and `+ship/release` naming one character by a plain name or `#n`.
#[must_use]
pub fn in_vocabulary(line: &str) -> bool {
    if line.len() > 64 || line.contains(['\n', '\r']) {
        return false;
    }
    if [CONDITION, EJECT, EXTRACT, CAPTIVES].contains(&line) {
        return true;
    }
    line.strip_prefix(RELEASE).is_some_and(|who| {
        let name = who.strip_prefix('#').unwrap_or(who);
        (1..=32).contains(&name.len())
            && if who.starts_with('#') {
                name.bytes().all(|b| b.is_ascii_digit())
            } else {
                name.bytes().all(|b| b.is_ascii_alphanumeric())
            }
    })
}

/// **A custody line the server said** (doc 42 Q-42-2's refusals).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Custody {
    /// Held aboard the named carrier.
    Held {
        /// The carrier's name.
        carrier: String,
    },
    /// Waiting on an ejection's pickup.
    Pickup,
    /// In processing at the named port.
    Processing {
        /// The port's name.
        port: String,
    },
}

/// **Read one line of the server's text as a custody line**, if it is one.
#[must_use]
pub fn custody(text: &str) -> Option<Custody> {
    let text = text.trim();
    if text == PICKUP {
        return Some(Custody::Pickup);
    }
    if let Some(carrier) = text
        .strip_prefix(HELD_HEAD)
        .and_then(|rest| rest.strip_suffix('.'))
    {
        return (!carrier.is_empty()).then(|| Custody::Held {
            carrier: carrier.to_owned(),
        });
    }
    let (port, _) = text.strip_prefix(PROCESSING_HEAD)?.split_once("; about ")?;
    (!port.is_empty()).then(|| Custody::Processing {
        port: port.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crew_lines_are_closed() {
        for line in [
            CONDITION,
            EJECT,
            EXTRACT,
            CAPTIVES,
            "+ship/release Ash",
            "+ship/release #42",
        ] {
            assert!(in_vocabulary(line), "{line}");
        }
        for line in [
            "+ship/release ",
            "+ship/release #4x",
            "+ship/release Ash Bel",
            "+ship/release Ash\nsay hi",
            "+sortie/extract go",
            "+ship/release",
        ] {
            assert!(!in_vocabulary(line), "{line}");
        }
    }

    /// The contract check: each reader matches the game's own sentence.
    #[test]
    fn the_custody_lines_match_the_game() {
        use tacsim::field::crew;
        assert_eq!(
            custody(&crew::HELD.replace("{carrier}", "Kestrel")),
            Some(Custody::Held {
                carrier: "Kestrel".to_owned()
            })
        );
        assert_eq!(custody(crew::PICKUP), Some(Custody::Pickup));
        assert_eq!(
            custody(
                &crew::PROCESSING
                    .replace("{port}", "Ceres Freeport")
                    .replace("{h}", "1")
            ),
            Some(Custody::Processing {
                port: "Ceres Freeport".to_owned()
            })
        );
        assert_eq!(custody("You are held aboard the ."), None);
        assert_eq!(custody(crew::NOT_STRANDED), None);
    }
}
