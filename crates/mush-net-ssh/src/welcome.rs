//! Classic welcome-screen command parsing (design 02 §5.3, M1 subset).
//!
//! `ssh play@host` (and `guest`/`new`, which alias to the same screen in
//! M1) lands on the banner plus these in-band commands. Passwords are the
//! rest of the line, so they may contain spaces; they are **not** masked —
//! `connect <account> <password>` typed in the clear is the classic front
//! door (the SSH transport encrypts the wire).

/// One parsed welcome-screen line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WelcomeCmd<'a> {
    /// `connect <account> <password>` (also `co`, `con`).
    Connect {
        /// Account name.
        account: &'a str,
        /// Password, verbatim (may contain spaces).
        password: &'a str,
    },
    /// `create <account> <password>`.
    Create {
        /// Desired account name.
        account: &'a str,
        /// Desired password, verbatim.
        password: &'a str,
    },
    /// `WHO`.
    Who,
    /// `quit` / `QUIT` / `logout`.
    Quit,
    /// `help` / `?`.
    Help,
    /// Blank line.
    Empty,
    /// Right verb, wrong shape; carries a usage string to print.
    Malformed {
        /// Usage line for the verb.
        usage: &'static str,
    },
    /// Anything else; carries the offending verb.
    Unknown(&'a str),
}

const CONNECT_USAGE: &str = "Usage: connect <account> <password>";
const CREATE_USAGE: &str = "Usage: create <account> <password>";

/// Parse one line typed at the welcome screen. Verbs are case-insensitive.
pub fn parse_welcome(line: &str) -> WelcomeCmd<'_> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return WelcomeCmd::Empty;
    }
    let (verb, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((v, r)) => (v, r.trim_start()),
        None => (trimmed, ""),
    };
    let verb_lc = verb.to_ascii_lowercase();
    match verb_lc.as_str() {
        "connect" | "con" | "co" => match split_account_password(rest) {
            Some((account, password)) => WelcomeCmd::Connect { account, password },
            None => WelcomeCmd::Malformed {
                usage: CONNECT_USAGE,
            },
        },
        "create" => match split_account_password(rest) {
            Some((account, password)) => WelcomeCmd::Create { account, password },
            None => WelcomeCmd::Malformed {
                usage: CREATE_USAGE,
            },
        },
        "who" => WelcomeCmd::Who,
        "quit" | "logout" => WelcomeCmd::Quit,
        "help" | "?" => WelcomeCmd::Help,
        _ => WelcomeCmd::Unknown(verb),
    }
}

/// First word is the account; everything after it (trimmed) is the
/// password. Both must be non-empty.
fn split_account_password(rest: &str) -> Option<(&str, &str)> {
    let (account, password) = rest.split_once(char::is_whitespace)?;
    let password = password.trim();
    if account.is_empty() || password.is_empty() {
        return None;
    }
    Some((account, password))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_table() {
        use WelcomeCmd::*;
        let cases: &[(&str, WelcomeCmd<'_>)] = &[
            (
                "connect nick hunter2",
                Connect {
                    account: "nick",
                    password: "hunter2",
                },
            ),
            (
                "CONNECT Nick Hunter2",
                Connect {
                    account: "Nick",
                    password: "Hunter2",
                },
            ),
            (
                "co nick hunter2",
                Connect {
                    account: "nick",
                    password: "hunter2",
                },
            ),
            (
                "con nick hunter2",
                Connect {
                    account: "nick",
                    password: "hunter2",
                },
            ),
            // Password keeps its internal spaces.
            (
                "connect nick correct horse battery",
                Connect {
                    account: "nick",
                    password: "correct horse battery",
                },
            ),
            (
                "  connect   nick   pw  ",
                Connect {
                    account: "nick",
                    password: "pw",
                },
            ),
            (
                "create zoe pw123",
                Create {
                    account: "zoe",
                    password: "pw123",
                },
            ),
            (
                "connect nick",
                Malformed {
                    usage: CONNECT_USAGE,
                },
            ),
            (
                "connect",
                Malformed {
                    usage: CONNECT_USAGE,
                },
            ),
            (
                "create solo",
                Malformed {
                    usage: CREATE_USAGE,
                },
            ),
            ("WHO", Who),
            ("who", Who),
            ("quit", Quit),
            ("QUIT", Quit),
            ("logout", Quit),
            ("help", Help),
            ("?", Help),
            ("", Empty),
            ("   ", Empty),
            ("frobnicate now", Unknown("frobnicate")),
            ("connectx a b", Unknown("connectx")),
        ];
        for (line, want) in cases {
            assert_eq!(&parse_welcome(line), want, "line {line:?}");
        }
    }
}
