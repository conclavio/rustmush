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
//!
//! **The commander's keys** (cut AJ.3; [`super::commander`]): `commander`
//! switches the optional model commander on, `commander_ms` is how long after
//! each answer it asks again (1 000 ms is the machines' own decision interval,
//! the fastest it is worth asking; the default 1 500 ms sits above the
//! measured median answer of the owner's model on default routing),
//! `commander_max_calls` caps the calls of one match, and the model is
//! `[arena.commander_model]` — or the session's own only with
//! `commander_from_session = true`, off by default because a `--goal` user's
//! `[model]` may be slow or a reasoning model (measured: a median 10.6 s, and
//! five answers in eight spent the whole budget thinking and said nothing), and
//! would be paid for up to `commander_max_calls` discarded answers a match.
use serde::{Deserialize, Serialize};

use crate::{Result, config::Model};

/// The `[arena]` table.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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
    /// The model commander sets the pilot's doctrine when a commander model
    /// resolves (cut AJ.3).
    pub commander: bool,
    /// The commander asks again this long after its last answer or error
    /// (1 000..=10 000).
    pub commander_ms: u64,
    /// The most commander calls in one match (1..=10 000).
    pub commander_max_calls: u32,
    /// With no `[arena.commander_model]`, the session's own model commands.
    /// Off by default: see the module head.
    pub commander_from_session: bool,
    /// `[arena.commander_model]`: the commander's own model, recorded in the
    /// cost ledger under the reserved profile name `arena_commander`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commander_model: Option<Model>,
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
            commander: true,
            commander_ms: 1_500,
            commander_max_calls: 400,
            commander_from_session: false,
            commander_model: None,
        }
    }
}

impl Settings {
    /// Refuse any key outside its range, and a commander model that
    /// [`Model::validate`] refuses (with its own sentence).
    pub fn validate(&self) -> Result<()> {
        if !(1..=100).contains(&self.matches)
            || !(1..=4).contains(&self.max_lines_per_s)
            || self.lead_start_grace_ms > 60_000
            || !(10..=3_600).contains(&self.wait_start_s)
            || !(150..=900).contains(&self.band_m)
            || !(2_000..=60_000).contains(&self.step_timeout_ms)
            || !(1_000..=10_000).contains(&self.commander_ms)
            || !(1..=10_000).contains(&self.commander_max_calls)
        {
            return Err("invalid [arena] settings (see mush-bot.toml.example)".into());
        }
        if let Some(model) = &self.commander_model {
            model.validate()?;
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
            Settings {
                commander_ms: 999,
                ..Settings::default()
            },
            Settings {
                commander_ms: 10_001,
                ..Settings::default()
            },
            Settings {
                commander_max_calls: 0,
                ..Settings::default()
            },
            Settings {
                commander_max_calls: 10_001,
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
                commander_ms: 1_000,
                commander_max_calls: 1,
                ..Settings::default()
            },
            Settings {
                matches: 1,
                max_lines_per_s: 4,
                lead_start_grace_ms: 60_000,
                wait_start_s: 3_600,
                band_m: 900,
                step_timeout_ms: 60_000,
                commander_ms: 10_000,
                commander_max_calls: 10_000,
                ..Settings::default()
            },
        ] {
            assert!(edge.validate().is_ok());
        }
    }

    #[test]
    fn the_commander_keys_default_and_the_commander_model_reads_as_a_table() {
        let defaults = Settings::default();
        assert!(defaults.commander);
        assert_eq!(defaults.commander_ms, 1_500);
        assert_eq!(defaults.commander_max_calls, 400);
        assert!(
            !defaults.commander_from_session,
            "the session model only by opt-in"
        );
        assert!(defaults.commander_model.is_none());
        let read: Settings = toml::from_str(
            "commander_ms = 2000\n\
             [commander_model]\n\
             endpoint = \"https://openrouter.ai/api/v1/chat/completions\"\n\
             name = \"deepseek/deepseek-v4.1-flash\"\n\
             max_tokens = 200\n\
             reasoning = { enabled = false }\n\
             provider = { order = [\"Together\"], allow_fallbacks = false }\n",
        )
        .expect("a table");
        assert!(read.validate().is_ok());
        let model = read.commander_model.expect("the commander model");
        assert_eq!(model.name, "deepseek/deepseek-v4.1-flash");
        assert_eq!(
            serde_json::Value::Object(model.reasoning.expect("reasoning")),
            serde_json::json!({"enabled": false})
        );
        // A commander model the model rules refuse is refused with their sentence.
        let bad: Settings =
            toml::from_str("[commander_model]\nendpoint = \"ftp://example.org\"\nname = \"m\"\n")
                .expect("a table");
        assert_eq!(
            bad.validate().expect_err("refused").to_string(),
            "invalid model endpoint/settings; use api_key_env for credentials"
        );
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
