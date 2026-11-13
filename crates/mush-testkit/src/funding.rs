//! The funded-accounts block (doc 46 cut CT.2).
//!
//! An owner test opens its fixture accounts through the real staff verbs
//! (`@instrument/register`, `@account/system`, `@account/open`, `@econ/mint`),
//! in an order that fixes the account ids the test then names: the faucet is
//! account 0, the first account opened after it is 1, and so on. Every rig
//! wrote that as a list of command strings, a loop that ran each as God, and a
//! hand-rolled scan of the output for an error span.
//!
//! [`Funding`] is that block: the lines in the order given, each run as God,
//! the first refusal an error that names the line and what it printed. It
//! knows the verbs' syntax and nothing of Economy's types, so a crate's own
//! unit tests can use it (this crate depends on no commerce crate).
//!
//! ```
//! use mush_core::ObjId;
//! use mush_testkit::funding::Funding;
//!
//! let funding = Funding::instrument("cr", 0)
//!     .system("faucet")
//!     .open(ObjId(2))
//!     .mint(0, 1, 10_000);
//! assert_eq!(
//!     funding.lines(),
//!     [
//!         "@instrument/register cr=0",
//!         "@account/system faucet=cr",
//!         "@account/open #2=cr",
//!         "@econ/mint 0=1 10000",
//!     ]
//! );
//! ```

use std::error::Error;
use std::fmt;

use mush_core::{GOD, ObjId};

use crate::cold::Boot;
use crate::fixtures::{has_error, texts};
use crate::{TestWorld, fold_module_ops_into};

/// The setup commands that open and fund a rig's accounts, in order.
#[derive(Clone, Debug)]
pub struct Funding {
    instrument: String,
    lines: Vec<String>,
}

/// A setup line that was refused: the fixture is broken, not the code under
/// test.
#[derive(Debug)]
pub struct SetupRefused {
    line: String,
    said: String,
}

impl fmt::Display for SetupRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fixture setup `{}` was refused: {}",
            self.line, self.said
        )
    }
}

impl Error for SetupRefused {}

impl Funding {
    /// Start with `@instrument/register <code>=<scale>`; the accounts after it
    /// are opened in that instrument.
    #[must_use]
    pub fn instrument(code: &str, scale: u8) -> Self {
        Self {
            instrument: code.to_owned(),
            lines: vec![format!("@instrument/register {code}={scale}")],
        }
    }

    /// `@account/system <name>=<instrument>`: a system source or sink.
    #[must_use]
    pub fn system(self, name: &str) -> Self {
        let line = format!("@account/system {name}={}", self.instrument);
        self.line(line)
    }

    /// `@account/open #<owner>=<instrument>`: a personal account.
    #[must_use]
    pub fn open(self, owner: ObjId) -> Self {
        let line = format!("@account/open #{}={}", owner.0, self.instrument);
        self.line(line)
    }

    /// `@account/open <owner>=<instrument>` for an owner written out, such as
    /// `org:Underwriter/0`.
    #[must_use]
    pub fn open_for(self, owner: &str) -> Self {
        let line = format!("@account/open {owner}={}", self.instrument);
        self.line(line)
    }

    /// `@econ/mint <source>=<to> <amount>`: mint from system account `source`
    /// into account `to`, both by account id.
    #[must_use]
    pub fn mint(self, source: u64, to: u64, amount: i64) -> Self {
        self.line(format!("@econ/mint {source}={to} {amount}"))
    }

    /// Any other setup line, in its place in the order (`@org/create …`,
    /// `@asset/title …`).
    #[must_use]
    pub fn line(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    /// The lines as they will be run.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Run every line as God.
    ///
    /// # Errors
    ///
    /// The first line that answered with an error, and what it printed. The
    /// lines after it are not run.
    pub fn run(&self, tw: &mut TestWorld) -> Result<(), SetupRefused> {
        for line in &self.lines {
            let events = tw.run(GOD, line);
            if has_error(&events) {
                return Err(SetupRefused {
                    line: line.clone(),
                    said: texts(&events),
                });
            }
        }
        Ok(())
    }

    /// [`Funding::run`], then what the setup wrote folded into `boot` and the
    /// world's dirty set drained: the rig's store now holds the funded
    /// accounts, and the next drain holds only what the test does.
    ///
    /// # Errors
    ///
    /// As [`Funding::run`]; nothing is folded on a refusal.
    pub fn run_into(&self, tw: &mut TestWorld, boot: &mut Boot) -> Result<(), SetupRefused> {
        self.run(tw)?;
        fold_module_ops_into(boot, tw.take_module_ops());
        let _ = tw.world_mut().take_dirty();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_are_the_staff_verbs_in_the_order_given() {
        let funding = Funding::instrument("cr", 0)
            .line("@org/create Underwriter")
            .system("faucet")
            .open(ObjId(3))
            .system("roll.registry")
            .open_for("org:Underwriter/0")
            .mint(0, 1, 10_000)
            .mint(0, 3, 250);
        assert_eq!(
            funding.lines(),
            [
                "@instrument/register cr=0",
                "@org/create Underwriter",
                "@account/system faucet=cr",
                "@account/open #3=cr",
                "@account/system roll.registry=cr",
                "@account/open org:Underwriter/0=cr",
                "@econ/mint 0=1 10000",
                "@econ/mint 0=3 250",
            ]
        );
    }

    #[test]
    fn a_refused_line_stops_the_setup_and_names_itself() {
        // No Economy module is composed, so the first verb is unknown.
        let mut tw = TestWorld::new();
        let mut boot = Boot::new();
        let refused = Funding::instrument("cr", 0)
            .system("faucet")
            .run_into(&mut tw, &mut boot)
            .expect_err("an unknown verb is refused");
        assert!(
            refused
                .to_string()
                .starts_with("fixture setup `@instrument/register cr=0` was refused: "),
            "{refused}"
        );
        assert!(boot.is_empty(), "nothing is folded on a refusal");
    }

    #[test]
    fn lines_that_are_not_refused_run_in_order() {
        let mut tw = TestWorld::new();
        let said = Funding {
            instrument: "cr".into(),
            lines: vec!["\"first".into(), "\"second".into()],
        };
        said.run(&mut tw).expect("say is never refused");
    }
}
