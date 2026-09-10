//! Server-side conversions from the raw `mush-config` sections into the typed
//! module configs. These live here (not in `mush-config`) so `mush-config` need
//! not depend on `mush-llm` / `mush-weather` / `mush-worldgen` / `ruleset-wod`.

use mush_config::config::{
    AtlasCfg, EconomyCfg, OnboardingCfg, ShadowrunCfg, TacsimCfg, WeatherCfg, WodCfg, WorldgenCfg,
};
use serde::Deserialize;

/// Re-exported so a host that contributes a declared cadence needs no direct
/// `mush-economy` dependency (the E6a addendum's `sr.month` contribution).
pub use mush_economy::DeclaredCadence;

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
        plan_cost_cap_usd: cfg.plan_cost_cap_usd,
        plan_cost_target_usd: cfg.plan_cost_target_usd,
        plan_max_rounds: cfg.plan_max_rounds,
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
    economy_config_with(cfg, &[])
}

/// [`economy_config`] with the **host-contributed** declared cadences a selected
/// ruleset needs (SP1, D-LBK-7; the E6a addendum).
///
/// A ruleset's campaign clock is not the operator's to remember: when
/// `[rulesets].enabled` names `shadowrun`, `sr.month` must exist as a declared
/// cadence or no lifestyle obligation can be created at all. So the host contributes
/// the stanza — **unless the operator declared one of that name themselves**, in
/// which case theirs stands untouched, exactly as `declared::adopt` leaves a live
/// clock alone. Nothing here rewrites a running clock; it only ensures the
/// declaration exists so the economy registers a provider for it at composition and
/// creates or adopts it at boot.
///
/// # Errors
///
/// Everything [`economy_config`] refuses.
pub fn economy_config_with(
    cfg: &EconomyCfg,
    contributed: &[(&str, mush_economy::DeclaredCadence)],
) -> Result<mush_economy::EconomyConfig, String> {
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
    for (name, decl) in contributed {
        // The operator's own declaration always wins — matched the way the cadence
        // table matches names (case-insensitively), so `SR.Month` is not a second clock.
        if cadences
            .keys()
            .any(|k: &smol_str::SmolStr| k.eq_ignore_ascii_case(name))
        {
            continue;
        }
        cadences.insert(smol_str::SmolStr::new(name), decl.clone());
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
        stale_contract_age_secs: cfg.stale_contract_age_secs,
        tx_tail_max: if cfg.tx_tail_max == 0 {
            mush_economy::DEFAULT_TX_TAIL_MAX
        } else {
            cfg.tx_tail_max
        },
        checkpoint_every_entries: cfg.checkpoint_every_entries,
        idempotency_window_secs: cfg.idempotency_window_secs,
        idempotency_window_max: if cfg.idempotency_window_max == 0 {
            mush_economy::DEFAULT_IDEMPOTENCY_WINDOW_MAX
        } else {
            cfg.idempotency_window_max
        },
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

/// The declared cadences a Shadowrun server contributes to `[economy.cadences]`
/// (the E6a addendum; §11 q1's defaults).
///
/// `sr.month` on the world's IC clock, a thirty-IC-day period, clock-level catch-up
/// 3 — the clock *forgives* on the clock; the lifestyle schedule's own
/// `maximum_catch_up` is 1 and decides *attempts* (D-E6-23, G12).
#[must_use]
pub fn shadowrun_cadences(
    cfg: &ruleset_shadowrun::SrEconomyConfig,
) -> Vec<(String, mush_economy::DeclaredCadence)> {
    if !cfg.enabled {
        return Vec::new();
    }
    vec![(
        cfg.lifestyle_clock.to_string(),
        mush_economy::DeclaredCadence {
            clock: mush_economy::DeclaredCadenceClock::WorldIc,
            period_secs: ruleset_shadowrun::SR_MONTH_PERIOD_SECS,
            catch_up: ruleset_shadowrun::SR_MONTH_CATCH_UP,
        },
    )]
}

/// The ruleset-facing `[shadowrun.economy]` config (E6a, 08f §9, answer 16).
///
/// Every word is validated here rather than defaulted, on `[rulesets].enabled`'s
/// rule: a silently-ignored typo is worse than not starting. A `lifestyle_costs` key
/// that is not a tier must not quietly become "this tier is free".
///
/// # Errors
///
/// An unknown lifestyle tier in `lifestyle_costs`, an empty `instrument` or
/// `lifestyle_clock`, a permille fence rate above 1000, or a non-positive
/// `board_ttl_secs`.
pub fn shadowrun_economy_config(
    cfg: &ShadowrunCfg,
) -> Result<ruleset_shadowrun::SrEconomyConfig, String> {
    let e = &cfg.economy;
    let instrument = e.instrument.trim();
    if instrument.is_empty() {
        return Err("[shadowrun.economy].instrument: an instrument needs a code".to_string());
    }
    let clock = e.lifestyle_clock.trim();
    if clock.is_empty() {
        return Err(
            "[shadowrun.economy].lifestyle_clock: a campaign clock needs a name".to_string(),
        );
    }
    for rate in [e.fence_rate_immediate, e.fence_rate_consign] {
        if rate > 1_000 {
            return Err(format!(
                "[shadowrun.economy]: a fence rate is permille of the baseline; {rate} is above \
                 1000"
            ));
        }
    }
    if e.board_ttl_secs <= 0 {
        return Err(
            "[shadowrun.economy].board_ttl_secs: a listing needs a positive window".to_string(),
        );
    }
    let mut lifestyle_costs = if e.lifestyle_costs.is_empty() {
        ruleset_shadowrun::default_lifestyle_costs()
    } else {
        std::collections::BTreeMap::new()
    };
    for (tier, cost) in &e.lifestyle_costs {
        let t = ruleset_shadowrun::LifestyleTier::parse(tier).ok_or_else(|| {
            format!(
                "[shadowrun.economy.lifestyle_costs]: {tier:?} is not a lifestyle tier (use \
                 \"street\", \"squatter\", \"low\", \"middle\", \"high\" or \"luxury\")"
            )
        })?;
        lifestyle_costs.insert(t, *cost);
    }
    // E6b (OPEN 1): the source-controlled fence baseline, keyed on a catalog code.
    // A code must be non-empty and bounded, and a zero baseline is refused rather
    // than silently pricing an object at nothing.
    let mut fence_baseline: std::collections::BTreeMap<smol_str::SmolStr, u64> =
        std::collections::BTreeMap::new();
    for (code, price) in &e.fence_baseline {
        let code = code.trim();
        if code.is_empty() || code.len() > ruleset_shadowrun::MAX_CATALOG_CODE_BYTES {
            return Err(format!(
                "[shadowrun.economy.fence_baseline]: {code:?} is not a usable catalog code (1..={}                  bytes)",
                ruleset_shadowrun::MAX_CATALOG_CODE_BYTES
            ));
        }
        if *price == 0 {
            return Err(format!(
                "[shadowrun.economy.fence_baseline]: {code:?} has a zero baseline; remove the row                  instead — an item with no reference price cannot be quoted"
            ));
        }
        fence_baseline.insert(smol_str::SmolStr::new(code), *price);
    }
    Ok(ruleset_shadowrun::SrEconomyConfig {
        enabled: e.enabled,
        instrument: smol_str::SmolStr::new(instrument),
        lifestyle_clock: smol_str::SmolStr::new(clock),
        lifestyle: e.lifestyle,
        player_run_offers: e.player_run_offers,
        organization_run_offers: e.organization_run_offers,
        chargen_shopping: e.chargen_shopping,
        catalog_paths: e.catalog_paths.clone(),
        lifestyle_costs,
        quote_ttl_live: e.quote_ttl_live,
        quote_ttl_remote: e.quote_ttl_remote,
        fence_rate_immediate: e.fence_rate_immediate,
        fence_rate_consign: e.fence_rate_consign,
        fence_delay_secs: e.fence_delay_secs,
        fence_baseline,
        vendor_orgs: e.vendor_orgs.clone(),
        active_offer_cap: e.active_offer_cap,
        board_ttl_secs: e.board_ttl_secs,
        board_cap: e.board_cap,
        chargen_carryover_cap: e.chargen_carryover_cap,
    })
}

/// The most bytes one TacSim pack file this host will read.
///
/// Mirrors `ruleset_tacsim::MAX_PACK_BYTES`, checked **before** the file is read so an
/// enormous file is refused by name rather than pulled into memory and then rejected.
pub const MAX_TACSIM_PACK_BYTES: u64 = 512 * 1_024;

/// The layer-facing `[tacsim]` config (E7 cut 1, 10b §7 and §3.1's OPEN 1).
///
/// **The host reads the bytes; the ruleset owns parse, digest and validation.** This
/// function walks `pack_paths` and `seed_paths`, reads each entry — a file, or every
/// `*.json` in a directory, in sorted order so a boot is deterministic — and hands
/// `ruleset_tacsim::verify_sources` `(name, text)` pairs. An unreadable, oversized,
/// unparseable or digest-mismatched pack is a **hard boot error naming the file**,
/// which is 10b §3.1 invariant (iv) and the only place it can be enforced: only the
/// host can fail a boot.
///
/// This is deliberately the opposite arrangement to E6c's catalogues, where
/// `ruleset-shadowrun`'s own `SrCatalogSource::pack` reads a file at
/// `@market/catalog load` time. A catalogue is an optional staff-triggered load whose
/// failure must fail the *pack* and never the server; a TacSim pack is a boot
/// precondition. The two answers differ on purpose.
///
/// With `enabled = false` nothing is read at all: an unconfigured server does not
/// touch the filesystem for a layer it is not running.
///
/// # Errors
///
/// A message naming the file, for an I/O failure, a file past
/// [`MAX_TACSIM_PACK_BYTES`], a directory that cannot be listed, or anything
/// `ruleset_tacsim::verify_sources` refuses.
pub fn tacsim_config(cfg: &TacsimCfg) -> Result<ruleset_tacsim::TacsimConfig, String> {
    let mut packs = Vec::new();
    let mut seeds = Vec::new();
    if cfg.enabled {
        packs = read_pack_sources(&cfg.pack_paths, "[tacsim].pack_paths")?;
        seeds = read_pack_sources(&cfg.seed_paths, "[tacsim].seed_paths")?;
    }
    let typed = ruleset_tacsim::TacsimConfig {
        enabled: cfg.enabled,
        packs,
        seeds,
        corridor: cfg.corridor.trim().to_string(),
        active_domain_limit: cfg.active_domain_limit,
        ground_pulse_ms: cfg.ground_pulse_ms,
        space_pulse_ms: cfg.space_pulse_ms,
        // Parsed and inert (10b §7, OPEN 4). Deliberately **not** validated: a knob
        // that configures nothing must not be able to refuse a boot, and E6a set the
        // same precedent with `catalog_paths`/`quote_ttl_*`. The cut that reads a
        // field is the cut that validates it.
        time: ruleset_tacsim::TacsimTimeConfig {
            rate: cfg.time.rate,
            catch_up_journeys: cfg.time.catch_up_journeys.clone(),
            catch_up_production: cfg.time.catch_up_production.clone(),
            catch_up_fronts: cfg.time.catch_up_fronts.clone(),
            catch_up_domains: cfg.time.catch_up_domains.clone(),
        },
        // ---- E7r1: R4 — `[tacsim.recovery]`, LIVE and therefore VALIDATED ----
        //
        // Unlike `[tacsim.time]`, this block has a reader in the same pass that adds
        // it, so *the cut that reads a field is the cut that validates it* applies:
        // a nonsense policy refuses the boot here, at the host, rather than being
        // discovered at a player's first application.
        recovery: recovery_config(&cfg.recovery)?,
    };
    // Verify here, at the host, so the boot fails with the file's name in the message.
    // The check is pure and the module repeats it at wiring; two evaluations of a pure
    // function over the same bytes cannot disagree.
    ruleset_tacsim::verify_sources(&typed)?;
    Ok(typed)
}

/// The module-facing [`mush_atlas::AtlasConfig`] (S1, 14-spatial-implementation-plan
/// §3.1): `[atlas]`'s limits, and its `pack_paths` read **host-side** exactly as
/// `[tacsim].pack_paths` are, so an unreadable, oversized or mis-digested pack is a
/// hard boot error naming the file before any module runs.
///
/// # Errors
///
/// A pack file that cannot be read or does not verify, a directory that cannot be
/// listed, more than [`mush_atlas::MAX_CONFIGURED_PACKS`] packs, or a
/// `tile_max_cells` outside `1..=65536`.
pub fn atlas_config(cfg: &AtlasCfg, worldgen: &WorldgenCfg) -> Result<mush_atlas::AtlasConfig, String> {
    let sources = read_pack_sources(&cfg.pack_paths, "[atlas].pack_paths")?;
    if worldgen.hydration_strikes_to_zero == 0 {
        return Err("[worldgen].hydration_strikes_to_zero must be at least 1".to_string());
    }
    let typed = mush_atlas::AtlasConfig {
        tile_max_cells: cfg.tile_max_cells,
        nearest_room_cells: cfg.nearest_room_cells,
        dwell_floor_s: cfg.dwell_floor_s,
        real_cap_s: cfg.real_cap_s.clone(),
        // S3: the hydration knobs live under `[worldgen]` (plan §3.4) and reach the
        // atlas, which owns the standing.
        hydration_cap: worldgen.hydration_cap,
        hydration_strikes_to_zero: worldgen.hydration_strikes_to_zero,
        packs: sources
            .into_iter()
            .map(|s| mush_atlas::PackSource {
                name: s.name,
                bytes: s.bytes,
            })
            .collect(),
    };
    // Verified here, at the host, so the boot fails with the file's name in the
    // message; the module repeats the parse at wiring, and two evaluations of a pure
    // function over the same bytes cannot disagree.
    mush_atlas::verify_sources(&typed)?;
    Ok(typed)
}

/// **E7r1 R4 — `[tacsim.recovery]`, validated at the host.**
///
/// A disabled provider validates nothing but its own disabled-ness: the block may
/// then be as empty as it likes, which is what keeps a server that has never heard of
/// recovery bit-identical to one built before the residue pass. An **enabled** one
/// must name a pool account, at least one tier, a positive application cap, a
/// positive cooldown and a known insurance word — because each of those is read at a
/// grant, and a grant is not the place to discover the policy is nonsense.
///
/// # Errors
///
/// A message naming `[tacsim.recovery]` and the exact key, so the boot failure is
/// actionable without reading this function.
fn recovery_config(
    cfg: &mush_config::config::TacsimRecoveryCfg,
) -> Result<ruleset_tacsim::TacsimRecoveryConfig, String> {
    let insurance = cfg.insurance.trim().to_string();
    let tiers: Vec<String> = cfg
        .tiers
        .iter()
        .map(|tier| tier.trim().to_string())
        .filter(|tier| !tier.is_empty())
        .collect();
    if cfg.enabled {
        if cfg.pool_account == 0 {
            return Err(
                "[tacsim.recovery]: an enabled provider needs `pool_account` — a pool that \
                 names no account can never be exhausted OR funded, and 'funded, never \
                 infinite' is the whole rule"
                    .to_owned(),
            );
        }
        if tiers.is_empty() {
            return Err(
                "[tacsim.recovery]: an enabled provider needs at least one entry in `tiers`; \
                 a grant names one"
                    .to_owned(),
            );
        }
        if cfg.applications_per_principal == 0 {
            return Err(
                "[tacsim.recovery]: `applications_per_principal` is at least 1; zero would \
                 refuse every applicant silently"
                    .to_owned(),
            );
        }
        if cfg.cooldown_days == 0 {
            return Err(
                "[tacsim.recovery]: `cooldown_days` is at least 1 (10b §7's RECOVERY_COOLDOWN \
                 is 14)"
                    .to_owned(),
            );
        }
        if cfg.upkeep_per_period < 0 {
            return Err("[tacsim.recovery]: `upkeep_per_period` is not negative".to_owned());
        }
        if cfg.pool_floor < 0 {
            return Err("[tacsim.recovery]: `pool_floor` is not negative".to_owned());
        }
    }
    if tiers.len() > ruleset_tacsim::MAX_RECOVERY_TIERS {
        return Err(format!(
            "[tacsim.recovery]: `tiers` declares {} entries; this server takes at most {}",
            tiers.len(),
            ruleset_tacsim::MAX_RECOVERY_TIERS
        ));
    }
    if !ruleset_tacsim::RECOVERY_INSURANCE_WORDS.contains(&insurance.as_str()) {
        return Err(format!(
            "[tacsim.recovery]: `insurance` is one of {}; `{insurance}` is a policy nothing \
             applies",
            ruleset_tacsim::RECOVERY_INSURANCE_WORDS.join(" | ")
        ));
    }
    Ok(ruleset_tacsim::TacsimRecoveryConfig {
        enabled: cfg.enabled,
        pool_account: cfg.pool_account,
        pool_floor: cfg.pool_floor,
        tiers,
        applications_per_principal: cfg.applications_per_principal,
        cooldown_secs: i64::from(cfg.cooldown_days).saturating_mul(24 * 60 * 60),
        max_operational_assets: cfg.max_operational_assets,
        insurance,
        upkeep_per_period: cfg.upkeep_per_period,
    })
}

/// The most pack files this host will read for one `[tacsim]` path list.
///
/// A host mirror of `ruleset_tacsim::MAX_CONFIGURED_PACKS`, so the **listing** pass can
/// consult it (independent verify B5). `verify_sources` keeps its own check of the
/// combined `packs + seeds` count as the defence-in-depth repeat.
pub const MAX_TACSIM_CONFIGURED_PACKS: usize = ruleset_tacsim::MAX_CONFIGURED_PACKS;

/// Read every configured pack path into `(name, bytes)` pairs, deterministically.
///
/// **Two passes, and the order is the point** (independent verify B5). The cap whose
/// doc-comment says *"Boot reads every one of them, so the list is bounded"* used to be
/// applied by `verify_sources` **after** every configured file had already been read
/// into memory: 200 pack files cost 200 × up-to-512 KiB of resident bytes before the
/// boot refused, and the refusal printed the count that got materialized — inverting
/// this tree's own "bounded on records visited, not matched" discipline at the one
/// place it is a boot precondition. So the candidate list is accumulated first and the
/// cap is applied to it, and only then is anything read. Sorted-directory order
/// (OPEN 1) is unchanged: the listing pass is where the sort happens.
fn read_pack_sources(
    paths: &[String],
    label: &str,
) -> Result<Vec<ruleset_tacsim::PackSource>, String> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    for entry in paths {
        let path = std::path::Path::new(entry.trim());
        if path.is_dir() {
            let listing = std::fs::read_dir(path)
                .map_err(|error| format!("{label}: {} could not be listed: {error}", entry))?;
            // Sorted, so which pack "wins" a duplicate `(id, version)` never depends
            // on the order the filesystem happened to hand them back.
            let mut files: Vec<std::path::PathBuf> = Vec::new();
            for item in listing {
                let item = item
                    .map_err(|error| format!("{label}: {} could not be listed: {error}", entry))?;
                let candidate = item.path();
                if candidate.extension().is_some_and(|e| e == "json") {
                    files.push(candidate);
                }
            }
            files.sort();
            if files.len() > MAX_TACSIM_CONFIGURED_PACKS {
                return Err(format!(
                    "{label}: {} holds {} pack files; this server loads at most \
                     {MAX_TACSIM_CONFIGURED_PACKS}",
                    entry,
                    files.len()
                ));
            }
            candidates.extend(files);
        } else {
            candidates.push(path.to_path_buf());
        }
        if candidates.len() > MAX_TACSIM_CONFIGURED_PACKS {
            return Err(format!(
                "{label}: {} pack files are configured; this server loads at most \
                 {MAX_TACSIM_CONFIGURED_PACKS}",
                candidates.len()
            ));
        }
    }
    let section = label.split('.').next().unwrap_or(label);
    let mut out: Vec<ruleset_tacsim::PackSource> = Vec::with_capacity(candidates.len());
    for file in candidates {
        let (name, bytes) = read_pack_file(&file, section)?;
        out.push(ruleset_tacsim::PackSource { name, bytes });
    }
    Ok(out)
}

/// Read one pack file for `section`, refusing anything over
/// [`MAX_TACSIM_PACK_BYTES`] — the one size a pack file may be, whichever section
/// names it.
fn read_pack_file(path: &std::path::Path, section: &str) -> Result<(String, String), String> {
    let name = path.display().to_string();
    let meta = std::fs::metadata(path)
        .map_err(|error| format!("{section} {name} could not be read: {error}"))?;
    if meta.len() > MAX_TACSIM_PACK_BYTES {
        return Err(format!(
            "{section} {name} is larger than this server will read ({MAX_TACSIM_PACK_BYTES} bytes)"
        ));
    }
    let bytes = std::fs::read_to_string(path)
        .map_err(|error| format!("{section} {name} could not be read: {error}"))?;
    Ok((name, bytes))
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

    /// **E7 cut 1, the B lens: a malformed pack FILE refuses the boot and names it.**
    ///
    /// This is the opposite answer to a corrupt stored `tacsim:*` document, which is
    /// skipped loudly while the world boots (`ruleset_tacsim::store`'s table). Both are
    /// asserted, in different crates, on purpose: a file is the operator's declared
    /// intent and refusing to start over a typo they can fix is right; a stored document
    /// is this world's accumulated history and refusing to start over one row of it is
    /// not (10b §4 rule 2, §3.1 invariant (iv)).
    #[test]
    fn e7c1_boot_a_malformed_pack_file_refuses_the_boot_and_names_it() {
        let dir = std::env::temp_dir().join(format!(
            "e7c1-packs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let write = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, body).expect("writes");
            path.display().to_string()
        };

        // The good case first: the built-in fixture, written out and read back, boots.
        let core = write("tacsim_core.json", ruleset_tacsim::fixture::CORE_JSON);
        let corridor = write(
            "tacsim_corridor.json",
            ruleset_tacsim::fixture::CORRIDOR_JSON,
        );
        let good = TacsimCfg {
            enabled: true,
            pack_paths: vec![core.clone()],
            seed_paths: vec![corridor.clone()],
            corridor: ruleset_tacsim::fixture::CORRIDOR_PACK.to_string(),
            ..TacsimCfg::default()
        };
        let typed = tacsim_config(&good).expect("the fixture packs load");
        assert_eq!(typed.packs.len(), 1);
        assert_eq!(typed.seeds.len(), 1);

        // A file that does not exist, one that is not a pack, one with an unknown key,
        // and one whose declared digest was computed over other bytes: every message
        // names **the file**.
        let missing = dir.join("nope.json").display().to_string();
        let garbage = write("garbage.json", "{ this is not json");
        let unknown_key = write(
            "unknown.json",
            &ruleset_tacsim::fixture::CORE_JSON.replacen(
                "\"entries\"",
                "\"surprise\": 1, \"entries\"",
                1,
            ),
        );
        let bad_digest = write(
            "bad_digest.json",
            &ruleset_tacsim::fixture::CORE_JSON.replacen(
                "940dab492e7c1bae4c51b0230149a6668dceb1d46aa4a2863a29ea52ce463759",
                "0000000000000000000000000000000000000000000000000000000000000000",
                1,
            ),
        );
        for (path, needle) in [
            (&missing, "could not be read"),
            (&garbage, "not a pack"),
            (&unknown_key, "unknown field"),
            (&bad_digest, "not this pack's digest"),
        ] {
            let cfg = TacsimCfg {
                enabled: true,
                pack_paths: vec![path.clone()],
                ..TacsimCfg::default()
            };
            let refused = tacsim_config(&cfg).expect_err(path);
            assert!(refused.contains(path), "the file must be named: {refused}");
            assert!(refused.contains(needle), "{path}: {refused}");
        }

        // A missing `requires`, and a `requires` version mismatch: both fail closed,
        // both name the file that declared the dependency.
        let seed_only = TacsimCfg {
            enabled: true,
            seed_paths: vec![corridor.clone()],
            ..TacsimCfg::default()
        };
        let refused = tacsim_config(&seed_only).expect_err("a missing dependency");
        assert!(refused.contains(&corridor), "{refused}");
        assert!(refused.contains("does not load"), "{refused}");
        let core_v2 = write(
            "core_v2.json",
            &ruleset_tacsim::fixture::CORE_JSON.replacen("\"version\": 1", "\"version\": 2", 1),
        );
        let mismatch = TacsimCfg {
            enabled: true,
            pack_paths: vec![core_v2.clone()],
            seed_paths: vec![corridor.clone()],
            ..TacsimCfg::default()
        };
        let refused = tacsim_config(&mismatch).expect_err("a version mismatch");
        // The v2 file's own digest no longer matches, which is itself the right
        // refusal — a version bump is new content and needs a new digest.
        assert!(refused.contains(&core_v2), "{refused}");

        // `enabled = false` reads nothing at all: even a path that does not exist is
        // untouched, because an unconfigured server does not go to the filesystem for a
        // layer it is not running.
        let off = TacsimCfg {
            pack_paths: vec![missing.clone()],
            ..TacsimCfg::default()
        };
        let typed = tacsim_config(&off).expect("an off layer reads nothing");
        assert!(typed.packs.is_empty() && !typed.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory in `pack_paths` is searched, in **sorted** order, so which pack wins
    /// a duplicate `(id, version)` never depends on what the filesystem happened to
    /// hand back — and a genuine duplicate is refused naming both files.
    #[test]
    fn e7c1_boot_a_pack_directory_is_read_in_sorted_order_and_a_duplicate_refuses() {
        let dir = std::env::temp_dir().join(format!("e7c1-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        std::fs::write(dir.join("a.json"), ruleset_tacsim::fixture::CORE_JSON).expect("writes");
        std::fs::write(dir.join("b.json"), ruleset_tacsim::fixture::CORE_JSON).expect("writes");
        let cfg = TacsimCfg {
            enabled: true,
            pack_paths: vec![dir.display().to_string()],
            ..TacsimCfg::default()
        };
        let refused = tacsim_config(&cfg).expect_err("two files declaring one version");
        assert!(
            refused.contains("a.json") && refused.contains("b.json"),
            "{refused}"
        );
        assert!(refused.contains("already declares"), "{refused}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- E7 cut 1 independent verify fold ----

    /// **B5 (covers S6) — the configured-pack cap is applied BEFORE any file is read.**
    ///
    /// `read_pack_sources` walked each configured directory with no bound on the file
    /// count and called `read_one_pack` for every `*.json` it found, each producing a
    /// `PackSource` holding the whole file, up to `MAX_TACSIM_PACK_BYTES`. Only
    /// afterwards did `verify_sources` count them and refuse past
    /// `MAX_CONFIGURED_PACKS` — so the cap whose doc-comment says *"Boot reads every one
    /// of them, so the list is bounded"* was applied after the reading it exists to
    /// bound, and the refusal printed the count that had already been materialized.
    ///
    /// The second half is what proves the ordering rather than merely the message: the
    /// same directory, filled with entries that **cannot be read at all**, still refuses
    /// with the cap. If anything were read first, the refusal would name the read
    /// failure instead.
    #[test]
    fn vf_b5_the_configured_pack_cap_is_applied_before_any_file_is_read() {
        let dir = std::env::temp_dir().join(format!(
            "e7c1-cap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let readable = dir.join("readable");
        let unreadable = dir.join("unreadable");
        std::fs::create_dir_all(&readable).expect("a scratch directory");
        std::fs::create_dir_all(&unreadable).expect("a scratch directory");
        for i in 0..=MAX_TACSIM_CONFIGURED_PACKS {
            std::fs::write(
                readable.join(format!("p{i:03}.json")),
                ruleset_tacsim::fixture::CORE_JSON,
            )
            .expect("writes");
            // A *directory* named `*.json`: it is listed as a candidate and cannot be
            // read, so reading it would produce "could not be read" rather than the cap.
            std::fs::create_dir_all(unreadable.join(format!("p{i:03}.json")))
                .expect("a scratch entry");
        }
        for (path, label) in [(&readable, "readable"), (&unreadable, "unreadable")] {
            let cfg = TacsimCfg {
                enabled: true,
                pack_paths: vec![path.display().to_string()],
                ..TacsimCfg::default()
            };
            let refused = tacsim_config(&cfg).expect_err(label);
            assert!(
                refused.contains(&path.display().to_string()),
                "{label}: the directory must be named: {refused}"
            );
            assert!(
                refused.contains(&(MAX_TACSIM_CONFIGURED_PACKS + 1).to_string())
                    && refused.contains(&MAX_TACSIM_CONFIGURED_PACKS.to_string()),
                "{label}: the count and the cap must both be named: {refused}"
            );
            assert!(
                !refused.contains("could not be read"),
                "{label}: nothing may have been read yet: {refused}"
            );
        }
        // And a directory inside the cap is still read, in sorted order, as before.
        let small = dir.join("small");
        std::fs::create_dir_all(&small).expect("a scratch directory");
        std::fs::write(small.join("core.json"), ruleset_tacsim::fixture::CORE_JSON)
            .expect("writes");
        let cfg = TacsimCfg {
            enabled: true,
            pack_paths: vec![small.display().to_string()],
            ..TacsimCfg::default()
        };
        assert_eq!(tacsim_config(&cfg).expect("one pack loads").packs.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
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
