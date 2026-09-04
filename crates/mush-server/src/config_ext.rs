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

/// The module-facing [`mush_economy::EconomyConfig`] (E4b1 verify fold, M1).
///
/// An absent `[economy]` section deserializes to `EconomyCfg::default()`, whose
/// values mirror the crate's, so a server with no economy configuration gets exactly
/// the crate defaults.
#[must_use]
pub fn economy_config(cfg: &EconomyCfg) -> mush_economy::EconomyConfig {
    mush_economy::EconomyConfig {
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
    }
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
        reserved_rooms: e.reserved_rooms.iter().copied().map(mush_core::ObjId).collect(),
        income,
    })
}
