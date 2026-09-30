//! The `[arena]` table of the bot's configuration (cut AJ.2).
//!
//! Every key has a default and a legal range, and [`Settings::validate`] refuses
//! anything outside it with one sentence. The two numbers that bound what the
//! pilot types are fixed by the owner's rulings, not by taste:
//!
//! * `max_lines_per_s` is at most **4** (16c Q-AJ4): the session's flood bucket
//!   refills at 10 lines a second and the cockpit queues at most 8 pending
//!   orders a tank, so four a second leaves room for a person at the same
//!   keyboard and never fills either;
//! * `band_m` is the orbit's radius. Its default, **450 m**, sits just outside
//!   the 420 m a machine closes to (`help simulator`, "Fighting the machines")
//!   and inside the 480 m a stopped tank on grass is seen at (`help cockpit`,
//!   "Seeing"), so the pilot keeps a stopped machine in view while making it
//!   turn. The range 150..=900 keeps it inside the stock gun's reach and outside
//!   a collision.
use serde::{Deserialize, Serialize};

use crate::Result;

/// The `[arena]` table.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// How many matches `--arena` plays before it reports (1..=100).
    pub matches: u32,
    /// The most lines the pilot types in any one second, every purpose
    /// together (1..=4).
    pub max_lines_per_s: u32,
    /// How long the side's lead waits after its claim before `SIM START`, the
    /// wait restarted by every other claim or give-up on the centre (0..=60 000).
    pub lead_start_grace_ms: u64,
    /// How long the pilot waits for a busy centre, for a lead to start the
    /// match, or for somebody to claim the other side (10..=3 600 seconds).
    pub wait_start_s: u64,
    /// The orbit's radius around the target, metres (150..=900).
    pub band_m: u32,
    /// How long one lobby line may go unanswered (2 000..=60 000).
    pub step_timeout_ms: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            matches: 1,
            max_lines_per_s: 4,
            lead_start_grace_ms: 3_000,
            wait_start_s: 300,
            band_m: 450,
            step_timeout_ms: 15_000,
        }
    }
}

impl Settings {
    /// Refuse any key outside its range.
    pub fn validate(&self) -> Result<()> {
        if !(1..=100).contains(&self.matches)
            || !(1..=4).contains(&self.max_lines_per_s)
            || self.lead_start_grace_ms > 60_000
            || !(10..=3_600).contains(&self.wait_start_s)
            || !(150..=900).contains(&self.band_m)
            || !(2_000..=60_000).contains(&self.step_timeout_ms)
        {
            return Err("invalid [arena] settings (see mush-bot.toml.example)".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn the_defaults_are_legal_and_each_bound_refuses() {
        let defaults = Settings::default();
        assert!(defaults.validate().is_ok());
        assert_eq!(defaults.max_lines_per_s, 4);
        assert_eq!(defaults.band_m, 450);
        let refused =
            |settings: Settings| settings.validate().expect_err("out of range").to_string();
        for settings in [
            Settings {
                matches: 0,
                ..Settings::default()
            },
            Settings {
                matches: 101,
                ..Settings::default()
            },
            Settings {
                max_lines_per_s: 5,
                ..Settings::default()
            },
            Settings {
                max_lines_per_s: 0,
                ..Settings::default()
            },
            Settings {
                lead_start_grace_ms: 60_001,
                ..Settings::default()
            },
            Settings {
                wait_start_s: 9,
                ..Settings::default()
            },
            Settings {
                wait_start_s: 3_601,
                ..Settings::default()
            },
            Settings {
                band_m: 149,
                ..Settings::default()
            },
            Settings {
                band_m: 901,
                ..Settings::default()
            },
            Settings {
                step_timeout_ms: 1_999,
                ..Settings::default()
            },
            Settings {
                step_timeout_ms: 60_001,
                ..Settings::default()
            },
        ] {
            assert_eq!(
                refused(settings),
                "invalid [arena] settings (see mush-bot.toml.example)"
            );
        }
        for edge in [
            Settings {
                matches: 100,
                max_lines_per_s: 1,
                lead_start_grace_ms: 0,
                wait_start_s: 10,
                band_m: 150,
                step_timeout_ms: 2_000,
            },
            Settings {
                matches: 1,
                max_lines_per_s: 4,
                lead_start_grace_ms: 60_000,
                wait_start_s: 3_600,
                band_m: 900,
                step_timeout_ms: 60_000,
            },
        ] {
            assert!(edge.validate().is_ok());
        }
    }

    #[test]
    fn the_table_reads_from_toml_and_refuses_an_unknown_key() {
        let read: Settings = toml::from_str("matches = 3\nband_m = 500\n").expect("a table");
        assert_eq!(read.matches, 3);
        assert_eq!(read.band_m, 500);
        assert_eq!(read.max_lines_per_s, 4);
        assert!(toml::from_str::<Settings>("lines_per_s = 2\n").is_err());
    }
}
