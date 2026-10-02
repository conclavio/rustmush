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
//! * `band_m` is the range the pilot holds (or orbits) at. Its default,
//!   **450 m**, sits just outside the 420 m a machine closes to (`help
//!   simulator`, "Fighting the machines") and inside the 480 m a stopped tank
//!   on grass is seen at (`help cockpit`, "Seeing"), so the pilot keeps a
//!   stopped machine in view. The range 150..=450 keeps it outside a collision
//!   and never past that sight (cut AK.1): the machines stop to shoot, so
//!   450 m, the stopped tank's 480 m less a margin for the scope's rounding and
//!   the tank's drift, is the ceiling of every band the pilot flies
//!   ([`SIGHT_BAND_MAX_M`]).
//!
//! **`engagement`** (cut AL.2) is how the pilot's own doctrine fights:
//! [`Engagement::Hold`] (the default) stops at the band with the bow on the
//! hostile and changes range only along the line of fire, ahead to close and
//! in reverse to open; [`Engagement::Orbit`] circles it, as the pilot did
//! before. `hold` is the default because **the facet you show decides what a
//! hit costs** (`help cockpit`, "Seeing": "from within 45 degrees of your bow
//! it strikes the front … a side is twice the target, 7 m of flank against
//! 3.5 m of bow", and the stock tank's hull carries 400 in front against 250 a
//! side and 150 behind, a side shot landing on that side's track three times
//! in ten — "Turn into the shot"), and because the orbit showed the machines a
//! side the whole fight: of the hits the bot took in its own measured run,
//! hull left 97, hull right 80, turret left 64, turret right 60, the two
//! tracks 61 and 58, and only 14 on the hull's front and 9 on the turret's.
//! The orbit stays a stance a commander or this key may ask for.
//!
//! **`opening` and `reapproach`** (cut AN.2; [`super::pilot`]'s module head has
//! the rules): `help simulator` says a machine that sees you "closes: to about
//! … 420 m …, where it stops, turns onto you and shoots", that "Open the range
//! past its sight and it comes after you again; let the ground hide you and it
//! drives at where it last saw you", and that one "that has seen nobody for a
//! minute and a half crosses to the other side's entry zone to look"; `help
//! cockpit` says "on grass a stopped tank can be seen out to 80 per cent of
//! that (480 m) and a moving one out to all of it", "a road hides nothing
//! either way", and "Stop before you are found". So `opening = "wait"` (the
//! default) holds stopped near the own entry, off the road, until a hostile is
//! in sight, and `"search"` drives for The Crossroads as the pilot did before;
//! `reapproach = true` backs away past a stopped machine's sight when it
//! out-hits the pilot, then waits for it to come on. **`reapproach` is off by
//! default** (cut AO.3): on thirty pooled matches from the muster only 11 of
//! its 31 back-offs reached their wait (its own reading had to show at least
//! half), so it ships off and stays a key. Any other `opening` word is
//! refused when the table is read.
//!
//! **`follow_broken` and `lattice`** (cut AN.3; [`super::pilot`]'s and
//! [`super::picture`]'s module heads have the rules): `help simulator` says a
//! machine "Hurt badly enough … breaks off for its own entry zone", fights on
//! the way home "only what is close (inside about 200 m)", and "after twenty
//! quiet seconds with nothing hostile in sight it turns round and comes back
//! in, once". So `follow_broken = true` (the default) follows such a machine
//! home from behind, outside 200 m, never into its entry zone, then backs out
//! of its sight and waits for its return. `lattice = true` (the default) fires
//! each gun on the arena's 250 ms lattice, read from the arena's own notices,
//! once that gun's cycle is learned and while the round trip is short;
//! `false` keeps the clock that adds a learned slack to each acknowledgement.
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

use super::{
    commander::Stance,
    pilot::{Knobs, SIGHT_BAND_MAX_M},
};
use crate::{Result, config::Model};

/// **How the pilot's own doctrine fights** (`[arena] engagement`, cut AL.2;
/// the module head says why `hold` is the default). Any other word is refused
/// when the table is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Engagement {
    /// Stop at the band, bow on ([`Stance::Hold`]).
    #[default]
    Hold,
    /// Circle the target at the band ([`Stance::Orbit`]).
    Orbit,
}

impl Engagement {
    /// The stance the pilot's own doctrine flies.
    pub fn stance(self) -> Stance {
        match self {
            Engagement::Hold => Stance::Hold,
            Engagement::Orbit => Stance::Orbit,
        }
    }
}

/// **How the pilot opens a match from the muster** (`[arena] opening`, cut
/// AN.2; the module head says why `wait` is the default). Any other word is
/// refused when the table is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Opening {
    /// Hold stopped near the own entry, off the road, bow toward the other
    /// side's entry, until a hostile is in sight, a hit or six minutes.
    #[default]
    Wait,
    /// Drive for The Crossroads after five quiet seconds (the pilot as it
    /// was).
    Search,
}

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
    /// The range the pilot holds, or orbits, at, metres (150..=450, the
    /// ceiling [`SIGHT_BAND_MAX_M`]).
    pub band_m: u32,
    /// How the pilot's own doctrine fights: `hold` (the default) or `orbit`
    /// (cut AL.2).
    pub engagement: Engagement,
    /// How the pilot opens from the muster: `wait` (the default) or `search`
    /// (cut AN.2).
    pub opening: Opening,
    /// Back away past a stopped machine's sight when it out-hits the pilot,
    /// then wait for it to come on (cut AN.2; off by default since cut AO.3:
    /// the module head says why).
    pub reapproach: bool,
    /// Follow a hurt machine driving away home from behind, never into its
    /// entry zone, then wait for its return (cut AN.3; on by default).
    pub follow_broken: bool,
    /// Fire on the arena's 250 ms lattice once each gun's cycle is learned
    /// (cut AN.3; on by default). The lane writes it into the picture
    /// ([`super::picture::Picture::set_lattice`]).
    pub lattice: bool,
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
            engagement: Engagement::Hold,
            opening: Opening::Wait,
            reapproach: false,
            follow_broken: true,
            lattice: true,
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
    /// **The pilot's knobs** (cut AN.2): `opening`, `reapproach` and (cut
    /// AN.3) `follow_broken` as the pilot reads them. `lattice` is the
    /// picture's, not the pilot's.
    pub fn knobs(&self) -> Knobs {
        Knobs {
            opening: self.opening,
            reapproach: self.reapproach,
            follow_broken: self.follow_broken,
        }
    }

    /// Refuse any key outside its range, and a commander model that
    /// [`Model::validate`] refuses (with its own sentence).
    pub fn validate(&self) -> Result<()> {
        if !(1..=100).contains(&self.matches)
            || !(1..=4).contains(&self.max_lines_per_s)
            || self.lead_start_grace_ms > 60_000
            || !(10..=3_600).contains(&self.wait_start_s)
            || !(150..=SIGHT_BAND_MAX_M).contains(&self.band_m)
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
                band_m: 451,
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
                band_m: 450,
                step_timeout_ms: 60_000,
                commander_ms: 10_000,
                commander_max_calls: 10_000,
                ..Settings::default()
            },
        ] {
            assert!(edge.validate().is_ok());
        }
    }

    /// **The band stops at the sight's ceiling** (cut AK.1): 450 m is legal,
    /// 451 m is refused with the table's one sentence.
    #[test]
    fn the_band_stops_at_the_sight() {
        assert_eq!(super::SIGHT_BAND_MAX_M, 450);
        let at = |band_m: u32| {
            Settings {
                band_m,
                ..Settings::default()
            }
            .validate()
        };
        assert!(at(450).is_ok());
        assert_eq!(
            at(451).expect_err("past the sight").to_string(),
            "invalid [arena] settings (see mush-bot.toml.example)"
        );
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

    /// **The knobs** (cut AN.2): `opening` reads `wait` and `search` and
    /// refuses any other word, `reapproach` reads a boolean, the defaults are
    /// `wait` and (cut AO.3) `false` (the pilot's [`Knobs::on`] with the
    /// re-approach off), and [`Settings::knobs`] says what the table says.
    #[test]
    fn the_knobs_read_and_refuse() {
        use super::{Knobs, Opening};
        let shipped = Knobs {
            reapproach: false,
            ..Knobs::on()
        };
        let defaults = Settings::default();
        assert_eq!(defaults.opening, Opening::Wait);
        assert!(!defaults.reapproach);
        assert_eq!(defaults.knobs(), shipped);
        let read: Settings = toml::from_str("").expect("a table");
        assert_eq!(read.knobs(), shipped, "left out: the defaults");
        let read: Settings = toml::from_str("reapproach = true\n").expect("a table");
        assert_eq!(read.knobs(), Knobs::on(), "the re-approach is still a key");
        // Cut AN.3: `Knobs::off` is the follow off too.
        let read: Settings =
            toml::from_str("opening = \"search\"\nreapproach = false\nfollow_broken = false\n")
                .expect("a table");
        assert_eq!(read.opening, Opening::Search);
        assert!(!read.reapproach);
        assert_eq!(read.knobs(), Knobs::off());
        assert!(read.validate().is_ok());
        let read: Settings = toml::from_str("opening = \"wait\"\n").expect("a table");
        assert_eq!(read.opening, Opening::Wait);
        for word in ["advance", "Wait", "hold", ""] {
            assert!(
                toml::from_str::<Settings>(&format!("opening = \"{word}\"\n")).is_err(),
                "{word}"
            );
        }
        assert!(toml::from_str::<Settings>("reapproach = \"yes\"\n").is_err());
    }

    /// **`follow_broken` and `lattice`** (cut AN.3): both booleans, both on by
    /// default; `follow_broken` is the pilot's knob, `lattice` is not.
    #[test]
    fn the_follow_and_the_lattice_read_and_refuse() {
        use super::Knobs;
        let defaults = Settings::default();
        assert!(defaults.follow_broken);
        assert!(defaults.lattice);
        let read: Settings = toml::from_str("").expect("a table");
        assert!(read.follow_broken && read.lattice, "left out: on");
        // Cut AO.3: the re-approach is set here, since its default is off.
        let read: Settings =
            toml::from_str("reapproach = true\nfollow_broken = false\nlattice = false\n")
                .expect("a table");
        assert!(!read.follow_broken && !read.lattice);
        assert_eq!(
            read.knobs(),
            Knobs {
                follow_broken: false,
                ..Knobs::on()
            }
        );
        assert!(read.validate().is_ok());
        assert!(toml::from_str::<Settings>("follow_broken = \"yes\"\n").is_err());
        assert!(toml::from_str::<Settings>("lattice = 1\n").is_err());
    }

    /// **`engagement`** (cut AL.2): `hold` by default, `orbit` read, and any
    /// other word refused — the commander's old `stand_off` included.
    #[test]
    fn engagement_defaults_to_hold_reads_orbit_and_refuses_anything_else() {
        use super::{Engagement, Stance};
        assert_eq!(Settings::default().engagement, Engagement::Hold);
        assert_eq!(Settings::default().engagement.stance(), Stance::Hold);
        let read: Settings = toml::from_str("").expect("a table");
        assert_eq!(read.engagement, Engagement::Hold, "left out: hold");
        let read: Settings = toml::from_str("engagement = \"hold\"\n").expect("a table");
        assert_eq!(read.engagement, Engagement::Hold);
        let read: Settings = toml::from_str("engagement = \"orbit\"\n").expect("a table");
        assert_eq!(read.engagement, Engagement::Orbit);
        assert_eq!(read.engagement.stance(), Stance::Orbit);
        assert!(read.validate().is_ok());
        for word in ["stand_off", "circle", "Hold", ""] {
            assert!(
                toml::from_str::<Settings>(&format!("engagement = \"{word}\"\n")).is_err(),
                "{word}"
            );
        }
    }
}
