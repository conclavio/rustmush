//! Server-side conversions from the raw `mush-config` sections into the typed
//! module configs. These live here (not in `mush-config`) so `mush-config` need
//! not depend on `mush-llm` / `mush-weather` / `mush-worldgen` / `ruleset-wod`.

use mush_config::config::{EconomyCfg, OnboardingCfg, WeatherCfg, WodCfg, WorldgenCfg};
use serde::Deserialize;

/// Deserialize the raw `[llm]` section into a [`mush_llm::LlmConfig`].
///
/// # Errors
///
/// A malformed `[llm]` section.
pub fn llm_config(raw: &toml::Value) -> Result<mush_llm::LlmConfig, String> {
    mush_llm::LlmConfig::deserialize(raw.clone()).map_err(|e| format!("[llm] config: {e}"))
}

/// The module-facing [`mush_weather::WeatherConfig`], overriding only the
/// config-surfaced knobs and keeping the crate defaults for the rest.
#[must_use]
pub fn weather_config(cfg: &WeatherCfg) -> mush_weather::WeatherConfig {
    mush_weather::WeatherConfig {
        flavor_variants: cfg.flavor_variants,
        ..mush_weather::WeatherConfig::default()
    }
}

/// The module-facing [`mush_worldgen::WorldgenConfig`].
#[must_use]
pub fn worldgen_config(cfg: &WorldgenCfg) -> mush_worldgen::WorldgenConfig {
    mush_worldgen::WorldgenConfig {
        batch_max_rooms: cfg.batch_max_rooms,
    }
}

/// The module-facing [`mush_economy::EconomyConfig`] (E4b1 verify fold, M1; SP1's
/// declared cadences, D-LBK-7).
///
/// An absent `[economy]` section deserializes to `EconomyCfg::default()`, whose
/// values mirror the crate's, so a server with no economy configuration gets exactly
/// the crate defaults.
///
/// `[economy.cadences]` is validated here on `[rulesets].enabled`'s rule — a
/// silently-ignored typo is worse than not starting: every `clock` word must be one
/// the economy can declare, every `catch_up` must be in the economy's band, and the
/// whole list must pass [`mush_economy::EconomyConfig::check_declared_cadences`]
/// (the count, the names, the periods).
///
/// # Errors
///
/// An unknown `clock` word, a `catch_up` outside `1..=8`, or a list the economy
/// refuses (a 33rd clock, a name that is not a canonical clock id, a non-positive
/// period).
pub fn economy_config(cfg: &EconomyCfg) -> Result<mush_economy::EconomyConfig, String> {
    let mut cadences = std::collections::BTreeMap::new();
    for (name, c) in &cfg.cadences {
        let clock = mush_economy::DeclaredCadenceClock::parse(&c.clock).ok_or_else(|| {
            format!(
                "[economy.cadences.{name:?}].clock: unknown clock {:?} (expected \"world_ic\" or \
                 \"real_utc\"; a manual clock is stepped by hand and cannot be declared)",
                c.clock
            )
        })?;
        let catch_up = u8::try_from(c.catch_up)
            .ok()
            .filter(|n| (1..=mush_economy::MAX_CATCH_UP_CAP).contains(n))
            .ok_or_else(|| {
                format!(
                    "[economy.cadences.{name:?}].catch_up: {} is outside 1..={}",
                    c.catch_up,
                    mush_economy::MAX_CATCH_UP_CAP
                )
            })?;
        cadences.insert(
            smol_str::SmolStr::new(name),
            mush_economy::DeclaredCadence {
                clock,
                period_secs: c.period_secs,
                catch_up,
            },
        );
    }
    let config = mush_economy::EconomyConfig {
        mint_obligation_cap_major: cfg.mint_obligation_cap_major,
        instruments: cfg
            .instruments
            .iter()
            .map(|(code, caps)| {
                (
                    code.clone(),
                    mush_economy::InstrumentCaps {
                        mint_obligation_cap_major: caps.mint_obligation_cap_major,
                    },
                )
            })
            .collect(),
        cadences,
    };
    config
        .check_declared_cadences()
        .map_err(|e| e.to_string())?;
    Ok(config)
}

/// The ruleset-facing onboarding config: resolve the room ids and fold the
/// `start_room_unapproved` sentinel back to `start_room`.
#[must_use]
pub fn onboarding_ruleset(cfg: &OnboardingCfg) -> ruleset_wod::OnboardingConfig {
    ruleset_wod::OnboardingConfig {
        chargen_required: cfg.chargen_required,
        start_room: cfg.start_room_id(),
        start_room_unapproved: cfg.start_room_unapproved_id(),
    }
}

/// The ruleset-facing `[wod.economy]` config (E5c, 08d §5.1, D-E5-9).
///
/// Every word is validated here rather than defaulted, on `[rulesets].enabled`'s
/// rule: a silently-ignored typo is worse than not starting. `personal_mode = "exat"`
/// must not quietly become `off` and leave a game-runner wondering why nobody has a
/// wallet.
///
/// # Errors
///
/// An unknown `personal_mode` or `default_engagement`, an income key that is not a
/// Resources dot `0..=5`, or an empty `cadence`.
pub fn wod_economy_config(cfg: &WodCfg) -> Result<ruleset_wod::WodEconomyConfig, String> {
    let e = &cfg.economy;
    let personal_mode = ruleset_wod::PersonalMode::parse(&e.personal_mode).ok_or_else(|| {
        format!(
            "[wod.economy].personal_mode: unknown mode {:?} (expected \"off\", \"abstract\" or \
             \"exact\")",
            e.personal_mode
        )
    })?;
    let default_engagement =
        ruleset_wod::Engagement::parse(&e.default_engagement).ok_or_else(|| {
            format!(
                "[wod.economy].default_engagement: unknown value {:?} (expected \"ambient\" or \
                 \"engaged\")",
                e.default_engagement
            )
        })?;
    let cadence = e.cadence.trim();
    if cadence.is_empty() {
        return Err("[wod.economy].cadence: a campaign clock needs a name".to_string());
    }
    let mut income = std::collections::BTreeMap::new();
    for (dot, amount) in &e.income {
        let n: u8 = dot.trim().parse().map_err(|_| {
            format!("[wod.economy.income]: {dot:?} is not a Resources dot (use \"0\"…\"5\")")
        })?;
        if n > 5 {
            return Err(format!(
                "[wod.economy.income]: {dot:?} is out of the Resources band (0…5)"
            ));
        }
        income.insert(n, *amount);
    }
    Ok(ruleset_wod::WodEconomyConfig {
        personal_mode,
        housing: e.housing,
        careers: e.careers,
        default_engagement,
        cadence: smol_str::SmolStr::new(cadence),
        starting_funds_major: e.starting_funds_major,
        reserved_rooms: e
            .reserved_rooms
            .iter()
            .copied()
            .map(mush_core::ObjId)
            .collect(),
        income,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn economy_cfg(toml_text: &str) -> EconomyCfg {
        toml::from_str::<mush_config::config::Config>(toml_text)
            .expect("the config parses")
            .economy
    }

    /// SP1 (D-LBK-7): `[economy.cadences]` converts into the economy's typed
    /// declarations with `create_cadence`'s defaults, and an absent table declares
    /// nothing.
    #[test]
    fn sp1_economy_cadences_convert_with_defaults() {
        assert!(
            economy_config(&EconomyCfg::default())
                .expect("defaults convert")
                .cadences
                .is_empty()
        );
        let cfg = economy_cfg(
            "[economy.cadences.\"chronicle.period\"]\nperiod_secs = 2592000\n\n\
             [economy.cadences.\"sr.month\"]\nclock = \"real_utc\"\nperiod_secs = 604800\n\
             catch_up = 5\n",
        );
        let typed = economy_config(&cfg).expect("a well-formed list converts");
        assert_eq!(typed.cadences.len(), 2);
        let chronicle = &typed.cadences["chronicle.period"];
        assert_eq!(chronicle.clock, mush_economy::DeclaredCadenceClock::WorldIc);
        assert_eq!(chronicle.period_secs, 2_592_000);
        assert_eq!(chronicle.catch_up, mush_economy::DEFAULT_CATCH_UP);
        let month = &typed.cadences["sr.month"];
        assert_eq!(month.clock, mush_economy::DeclaredCadenceClock::RealUtc);
        assert_eq!((month.period_secs, month.catch_up), (604_800, 5));
    }

    /// SP1 (D-LBK-7): a bad stanza refuses to start — a 33rd clock, a name that is
    /// not a canonical clock id, an unknown clock word (`manual` included: a manual
    /// clock is stepped by hand), a non-positive period, a catch-up outside the band.
    #[test]
    fn sp1_economy_cadences_refuse_33_clocks_and_a_bad_stanza() {
        let mut many = String::new();
        for i in 0..33 {
            many.push_str(&format!("[economy.cadences.c{i}]\nperiod_secs = 10\n"));
        }
        let refused = economy_config(&economy_cfg(&many)).expect_err("33 clocks");
        assert!(refused.contains("33"), "{refused}");
        let cases: [(&str, &str); 6] = [
            (
                "[economy.cadences.\"Chronicle\"]\nperiod_secs = 10\n",
                "canonical clock id",
            ),
            (
                "[economy.cadences.\"a..b\"]\nperiod_secs = 10\n",
                "canonical clock id",
            ),
            (
                "[economy.cadences.season]\nclock = \"manual\"\nperiod_secs = 10\n",
                "unknown clock",
            ),
            (
                "[economy.cadences.season]\nclock = \"sidereal\"\nperiod_secs = 10\n",
                "unknown clock",
            ),
            (
                "[economy.cadences.season]\nperiod_secs = 0\n",
                "period_secs",
            ),
            (
                "[economy.cadences.season]\nperiod_secs = 10\ncatch_up = 9\n",
                "catch_up",
            ),
        ];
        for (text, needle) in cases {
            let refused = economy_config(&economy_cfg(text)).expect_err(text);
            assert!(refused.contains(needle), "{text}: {refused}");
        }
    }
}
