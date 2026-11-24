//! **Game figures, read and never restated** (doc 51 §3.1, rule F).
//!
//! A story that waits "one cannon cycle" or fires "the rack's load less one"
//! holds a game figure. Written as a literal, the figure is a second copy of
//! the content, and the next tuning pass breaks the story: a 12-s wait written
//! for a 1-s cycle failed for a week after the cycle moved to 15 s
//! (`docs/flaky-tests.md`, rows 3 and 9). [`Figures`] reads the figure from
//! the catalog the story's server binds its units through and from the
//! [`TacsimConfig`] the story booted it with, so the two cannot disagree.
//!
//! It also holds the dice a pinned seed gives (doc 51 §3.2, rule D). Every
//! draw of a match is a keyed hash of the match seed and a label, with no
//! stream, so a story can ask before it boots what a given shot will roll and
//! choose a seed whose roll makes the outcome certain ([`Figures::seed_where`]).

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use mush_core::CanonicalId;
use tacsim::TacsimConfig;
use tacsim::arena::{DrawLabel, draw, draw_permille};
use tacsim::config::{MatchConfig, PackSource, TempoConfig};
use tacsim::definitions::unit::{COMPONENT_HITS, ComponentRole};
use tacsim::definitions::{Catalog, ResolutionDef, UnitSpec};

use crate::wait::loaded;

/// How many seeds [`Figures::seed_where`] tries before it gives up.
const SEED_SEARCH: u64 = 1_000_000;

/// The shipped catalog, bound once a process: the one the production
/// composition binds `SIM UNIT` and `@unit/create` through.
fn shipped() -> Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    Arc::clone(CATALOG.get_or_init(|| {
        Arc::new(
            tacsim::builtin_definitions()
                .unwrap_or_else(|error| panic!("the shipped content does not bind: {error}")),
        )
    }))
}

/// **The catalog a server booted with `config` plays**, built the way the
/// server builds it (`tacsim::composition::tactics_catalog_with`): the shipped
/// one, or the fielded catalog over the config's own packs when it names any.
/// A rig that adds a test sensor or a test unit therefore reads its figures
/// from the content it really booted. Bound once a set of packs.
fn catalog_of(config: &TacsimConfig) -> Arc<Catalog> {
    if config.packs.is_empty() {
        return shipped();
    }
    static BUILT: Mutex<Vec<(Vec<PackSource>, Arc<Catalog>)>> = Mutex::new(Vec::new());
    let mut built = BUILT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, catalog)) = built.iter().find(|(packs, _)| *packs == config.packs) {
        return Arc::clone(catalog);
    }
    let packs = tacsim::verify_sources(config)
        .unwrap_or_else(|error| panic!("the story's packs do not verify: {error}"));
    let catalog = Arc::new(
        Catalog::build_fielded(&packs, tacsim::NS)
            .unwrap_or_else(|error| panic!("the story's packs do not bind: {error}")),
    );
    built.push((config.packs.clone(), Arc::clone(&catalog)));
    catalog
}

/// One mount on a shipped unit, as its packs author it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mount {
    /// Its round's `count_per_load`: what a fresh unit carries.
    pub magazine: u32,
    /// Its `burst_rounds`: what one pull spends.
    pub burst: u32,
    /// Its `cycle_ms`: arena milliseconds from one pull to the next.
    pub cycle_ms: u32,
    /// Its `muzzle_speed_cms`. Zero is a beam, which lands as it fires.
    pub muzzle_speed_cms: u32,
    /// Its reach: the far edge of its last range band, in centimeters. A shot
    /// past it is not resolved at all.
    pub reach_cm: u32,
}

impl Mount {
    /// `N of N round(s)`: the full load a fresh unit's `WEAPONSTATUS` row reads.
    #[must_use]
    pub fn full(self) -> String {
        self.left(0)
    }

    /// `M of N round(s)` once `spent` rounds have gone.
    ///
    /// # Panics
    ///
    /// When `spent` is more than the load: a story's own arithmetic is wrong.
    #[must_use]
    pub fn left(self, spent: u32) -> String {
        let left = self
            .magazine
            .checked_sub(spent)
            .unwrap_or_else(|| panic!("{spent} round(s) spent from a load of {}", self.magazine));
        format!("{left} of {} round(s)", self.magazine)
    }

    /// How many times the mount fires before it is dry: the load over the burst.
    #[must_use]
    pub fn pulls(self) -> u32 {
        self.magazine / self.burst.max(1)
    }

    /// One cycle, in arena time.
    #[must_use]
    pub fn cycle(self) -> Duration {
        Duration::from_millis(u64::from(self.cycle_ms))
    }

    /// A round's time in the air over `range_m` meters: the range over the
    /// muzzle speed, as the arena schedules the impact. Nothing for a beam.
    #[must_use]
    pub fn flight(self, range_m: u32) -> Duration {
        self.flight_cm(u64::from(range_m) * 100)
    }

    /// The longest a round of this mount can be in the air: its flight over
    /// its whole reach.
    #[must_use]
    pub fn longest_flight(self) -> Duration {
        self.flight_cm(u64::from(self.reach_cm))
    }

    fn flight_cm(self, range_cm: u64) -> Duration {
        if self.muzzle_speed_cms == 0 {
            return Duration::ZERO;
        }
        Duration::from_millis(range_cm * 1_000 / u64::from(self.muzzle_speed_cms))
    }
}

/// **The figures of one story's server**: its content and its configuration.
///
/// ```
/// # use mush_storykit::Figures;
/// let config = tacsim::TacsimConfig::default();
/// let fig = Figures::of(&config);
/// let cannon = fig.mount("line_tank", "cannon");
/// // A wait for the cannon to be ready again is bounded by the content's own
/// // cycle, never by a number written in the story.
/// assert!(fig.after_one_round(cannon) >= cannon.cycle());
/// ```
#[derive(Clone, Debug)]
pub struct Figures {
    catalog: Arc<Catalog>,
    resolution: ResolutionDef,
    tempo: TempoConfig,
    match_: MatchConfig,
    sensor_pass_ms: u32,
}

impl Figures {
    /// The figures of a server booted with `config`: its tempo and its match
    /// block, over the catalog that config builds (the shipped content, and
    /// the config's own packs when it names any).
    ///
    /// # Panics
    ///
    /// When the config's packs do not verify or bind, as the server's boot
    /// would refuse them.
    #[must_use]
    pub fn of(config: &TacsimConfig) -> Figures {
        let catalog = catalog_of(config);
        Figures {
            resolution: tacsim::match_::resolution_for(&catalog),
            catalog,
            tempo: config.arena.tempo.clone(),
            match_: config.match_.clone(),
            sensor_pass_ms: config.sensors.pass_ms,
        }
    }

    /// The fielded unit `unit` (`line_tank`, `heavy_mk1`, ...).
    fn spec(&self, unit: &str) -> &UnitSpec {
        let key = CanonicalId::new(format!("tacsim.unit.{unit}"))
            .unwrap_or_else(|_| panic!("tacsim.unit.{unit} is not an id"));
        self.catalog
            .specs
            .get(&key)
            .unwrap_or_else(|| panic!("{key} is not a fielded unit"))
    }

    /// The mount `component` on the fielded unit `unit`: its load, its burst,
    /// its cycle, its muzzle speed and its reach. `component` is the name
    /// `WEAPONSTATUS` prints: `cannon`, `machine_gun`, `missiles`.
    ///
    /// # Panics
    ///
    /// When the unit is not fielded, or carries no such mount: a story's premise.
    #[must_use]
    pub fn mount(&self, unit: &str, component: &str) -> Mount {
        let spec = self.spec(unit);
        let weapon = spec
            .components
            .iter()
            .find_map(|slot| match &slot.role {
                ComponentRole::Weapon { weapon, .. } if slot.id.as_str() == component => {
                    Some(weapon)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("tacsim.unit.{unit} carries no mount {component}"));
        let def = self
            .catalog
            .weapons
            .get(weapon)
            .unwrap_or_else(|| panic!("{weapon} is not in the catalog"));
        let round = self
            .catalog
            .ammunition
            .get(&def.ammunition)
            .unwrap_or_else(|| panic!("{} is not in the catalog", def.ammunition));
        Mount {
            magazine: round.count_per_load,
            burst: def.burst_rounds,
            cycle_ms: def.cycle_ms,
            muzzle_speed_cms: def.muzzle_speed_cms,
            reach_cm: def
                .bands
                .iter()
                .map(|band| band.max_range_cm)
                .max()
                .unwrap_or_default(),
        }
    }

    /// **The arena's slowest wake** at the story's tempo: the longest of the
    /// three periods. With no staff floor a pulse never comes later than this.
    #[must_use]
    pub fn pulse(&self) -> Duration {
        self.pulse_under(0)
    }

    /// [`Figures::pulse`] under a staff floor of `floor_ms` (`@arena/tempo`).
    #[must_use]
    pub fn pulse_under(&self, floor_ms: u32) -> Duration {
        let slowest = self
            .tempo
            .quiet_ms
            .max(self.tempo.alert_ms)
            .max(self.tempo.engaged_ms)
            .max(floor_ms);
        Duration::from_millis(u64::from(slowest))
    }

    /// **The limit of a wait for one round**: the mount's cycle, the longest
    /// its round can fly and a pulse either side, under the load factor. A
    /// wait for the mount to read ready again, or for a round's outcome, is
    /// bounded by this. It is a limit, never a pace: the wait polls the
    /// cockpit and returns when the cockpit says so, and a tuning change makes
    /// the story slower, never red. It takes the whole reach, not the story's
    /// own range, so the story restates no distance either.
    #[must_use]
    pub fn after_one_round(&self, mount: Mount) -> Duration {
        loaded(mount.cycle() + mount.longest_flight() + self.pulse() * 2)
    }

    /// **The limit of a wait for `unit` to drive `meters`**: the time its
    /// mobility needs to reach its top speed, the distance at that speed and a
    /// pulse either side, under the load factor. A wait for two units to close
    /// a gap, or for one to reach a place, is bounded by this.
    ///
    /// # Panics
    ///
    /// When the unit is not fielded or authors no ground mobility.
    #[must_use]
    pub fn crossing(&self, unit: &str, meters: u32) -> Duration {
        let mobility = self.spec(unit).mobility;
        let drive = mobility
            .tracked
            .or(mobility.wheeled)
            .or(mobility.legged)
            .unwrap_or_else(|| panic!("tacsim.unit.{unit} authors no ground mobility"));
        let top = u64::from(drive.max_speed_cms.max(1));
        let to_speed_ms = top * 1_000 / u64::from(drive.accel_cms2.max(1));
        let at_speed_ms = u64::from(meters) * 100 * 1_000 / top;
        loaded(Duration::from_millis(to_speed_ms + at_speed_ms) + self.pulse() * 2)
    }

    /// **One sensor pass**, in the whole arena seconds `@arena/status` counts
    /// in (`[tacsim.sensors].pass_ms`, rounded up; at least one): how often a
    /// platform looks again. A story that waits "a few passes" waits this many
    /// arena seconds a pass ([`crate::arena_seconds`]).
    #[must_use]
    pub fn sensor_pass_seconds(&self) -> u64 {
        u64::from(self.sensor_pass_ms).div_ceil(1_000).max(1)
    }

    /// **The limit of a wait for a dropped pilot's tank to be theirs again**:
    /// the story's `reclaim_min_ms`, the floor under a `BOARD` that takes a
    /// tank back, and a pulse either side, under the load factor.
    #[must_use]
    pub fn after_reclaim(&self) -> Duration {
        loaded(Duration::from_millis(u64::from(self.match_.reclaim_min_ms)) + self.pulse() * 2)
    }

    /// The least and the most a shot in reach and in the open may be given,
    /// as probabilities: the resolution table's `p_min` and `p_max`.
    #[must_use]
    pub fn chance_bounds(&self) -> (f64, f64) {
        (
            f64::from(self.resolution.p_min) / 1_000.0,
            f64::from(self.resolution.p_max) / 1_000.0,
        )
    }

    // ---- the dice of a pinned seed (doc 51 §3.2) ---------------------------

    /// What the match's `shot`-th accepted shot rolls under `seed`, per mille:
    /// the arena's own `hit:<shot>` draw. A one-round pull lands when this is
    /// under its chance. Shots are numbered from 1 across the whole match, in
    /// the order the arena accepted them.
    #[must_use]
    pub fn hit_roll(&self, seed: u64, shot: u64) -> u32 {
        u32::from(draw_permille(seed, &DrawLabel::Hit { shot_seq: shot }))
    }

    /// Whether the `shot`-th shot **lands whatever its odds**: its roll is
    /// under the table's floor, so no range in reach and no aspect in the open
    /// can make it miss. For a one-round pull.
    #[must_use]
    pub fn sure_hit(&self, seed: u64, shot: u64) -> bool {
        self.hit_roll(seed, shot) < self.resolution.p_min
    }

    /// Whether the `shot`-th shot **misses whatever its odds**: its roll is at
    /// or over the table's ceiling. For a one-round pull.
    #[must_use]
    pub fn sure_miss(&self, seed: u64, shot: u64) -> bool {
        self.hit_roll(seed, shot) >= self.resolution.p_max
    }

    /// Whether the `shot`-th shot, if it reaches a section's internals of a
    /// `target` and leaves the section standing, **jams or damages a part**
    /// there (doc 36 cut GC.2): the arena's own `component:<shot>` draw against
    /// the unit's chances. A jammed gun leaves the weapon table, and a side
    /// with no gun left is out, so a duel can end on this with neither a wreck
    /// nor a turret shot off.
    #[must_use]
    pub fn harms_a_part(&self, target: &str, seed: u64, shot: u64) -> bool {
        let hits = self.spec(target).component_hits.unwrap_or(COMPONENT_HITS);
        let roll = draw(seed, &DrawLabel::Component { shot_seq: shot }) % 1_000;
        roll < u64::from(hits.jam_permille) + u64::from(hits.damage_permille)
    }

    /// **A chosen seed**: the first seed at or after `from` for which `holds`.
    /// It is how a story forces a roll with no door into the arena: say what
    /// the dice must do, and pin the seed that does it.
    ///
    /// ```
    /// # use mush_storykit::Figures;
    /// let fig = Figures::of(&tacsim::TacsimConfig::default());
    /// // The first round of the match lands, at any range and any aspect.
    /// let seed = fig.seed_where(1, |seed| fig.sure_hit(seed, 1));
    /// assert!(fig.sure_hit(seed, 1));
    /// ```
    ///
    /// # Panics
    ///
    /// When no seed of the next million holds: the story asks for dice the
    /// rules do not give.
    #[must_use]
    pub fn seed_where(&self, from: u64, holds: impl Fn(u64) -> bool) -> u64 {
        (0..SEED_SEARCH)
            .map(|step| from.wrapping_add(step))
            .find(|&seed| seed != 0 && holds(seed))
            .unwrap_or_else(|| {
                panic!("no seed in {SEED_SEARCH} from {from} gives the dice the story asks for")
            })
    }
}

/// **The dice seed a story's server opens its matches on.**
///
/// A story whose assertions lean on a draw passes the seed it chose
/// (`Some(seed)`), and the same dice fall on every run. A story that leans on
/// none passes `None` and gets a fresh seed, printed, so a failure that turns
/// out to depend on the dice after all names the seed that replays it. The
/// fresh seed is drawn once a process.
///
/// `STORY_SEED=<n>` in the environment overrides both, to replay a failure:
/// decimal, or hex with `0x`. The rig sets what this returns as
/// `TacsimConfig::arena.seed`, which nothing but a hand-built config can set.
///
/// # Panics
///
/// When `STORY_SEED` is set and is not a number.
#[must_use]
pub fn story_seed(pinned: Option<u64>) -> u64 {
    if let Ok(text) = std::env::var("STORY_SEED") {
        let text = text.trim();
        let parsed = match text.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16),
            None => text.parse::<u64>(),
        };
        let seed = parsed.unwrap_or_else(|_| panic!("STORY_SEED={text} is not a seed"));
        eprintln!("story seed: {seed} (from STORY_SEED)");
        return seed;
    }
    match pinned {
        Some(seed) => seed,
        None => {
            // One seed a process: a rig builds its config more than once (a
            // reboot over the same store, a second read for the figures).
            static FRESH: OnceLock<u64> = OnceLock::new();
            *FRESH.get_or_init(|| {
                let seed = fresh_seed();
                eprintln!("story seed: {seed} (fresh; replay with STORY_SEED={seed})");
                seed
            })
        }
    }
}

/// Sixty-four bits nobody chose, with no dependency for it: the standard
/// library seeds every `RandomState` from the OS.
fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let seed = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    if seed == 0 { 1 } else { seed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fig() -> Figures {
        Figures::of(&TacsimConfig::default())
    }

    /// The figures are the content's: each is checked against the typed
    /// catalog, never against a number written here.
    #[test]
    fn a_mount_reads_its_load_cycle_and_speed_off_the_catalog() {
        let fig = fig();
        let catalog = shipped();
        for (unit, component, weapon) in [
            ("line_tank", "cannon", "tacsim.weapon.cannon_105"),
            ("heavy_mk1", "missiles", "tacsim.weapon.missile_lr"),
        ] {
            let mount = fig.mount(unit, component);
            let def = &catalog.weapons[&CanonicalId::new(weapon).unwrap()];
            let round = &catalog.ammunition[&def.ammunition];
            assert_eq!(mount.cycle_ms, def.cycle_ms, "{unit} {component}");
            assert_eq!(mount.burst, def.burst_rounds, "{unit} {component}");
            assert_eq!(mount.muzzle_speed_cms, def.muzzle_speed_cms);
            assert_eq!(
                Some(mount.reach_cm),
                def.bands.iter().map(|band| band.max_range_cm).max()
            );
            assert_eq!(mount.magazine, round.count_per_load);
            assert_eq!(mount.pulls(), round.count_per_load / def.burst_rounds);
            assert_eq!(
                mount.full(),
                format!("{0} of {0} round(s)", round.count_per_load)
            );
            assert_eq!(
                mount.left(1),
                format!(
                    "{} of {} round(s)",
                    round.count_per_load - 1,
                    round.count_per_load
                )
            );
        }
    }

    /// A rig that boots with packs of its own reads the catalog those packs
    /// build: the shipped units, and its own beside them.
    #[test]
    fn a_config_with_packs_reads_the_catalog_they_build() {
        use tacsim::definitions::content;
        let renamed = content::LINE_TANK_JSON
            .replace("tacsim.pack.line_tank", "tacsim.pack.story_tank")
            .replace("tacsim.unit.line_tank", "tacsim.unit.story_tank");
        let mut document: serde_json::Value = serde_json::from_str(&renamed).unwrap();
        document["manifest"]["digest"] = serde_json::Value::String("0".repeat(64));
        let digest = mush_content::parse_pack(&document.to_string())
            .unwrap()
            .computed_digest()
            .unwrap()
            .to_hex();
        document["manifest"]["digest"] = serde_json::Value::String(digest);
        let mut config = TacsimConfig {
            packs: content::fielded_sources(),
            ..TacsimConfig::default()
        };
        config.packs.push(PackSource {
            name: "story_tank.json".to_owned(),
            bytes: document.to_string(),
        });
        let fig = Figures::of(&config);
        assert_eq!(
            fig.mount("story_tank", "cannon"),
            fig.mount("line_tank", "cannon"),
            "the rig's own unit carries the shipped cannon"
        );
        assert_eq!(
            fig.mount("line_tank", "cannon"),
            self::fig().mount("line_tank", "cannon")
        );
    }

    #[test]
    #[should_panic(expected = "carries no mount rack")]
    fn a_mount_the_unit_does_not_carry_is_the_storys_fault() {
        let _ = fig().mount("line_tank", "rack");
    }

    #[test]
    fn a_flight_is_the_range_over_the_muzzle_speed() {
        let mount = Mount {
            magazine: 4,
            burst: 1,
            cycle_ms: 30_000,
            muzzle_speed_cms: 25_000,
            reach_cm: 300_000,
        };
        // 1,100 m at 250 m/s, and the whole 3 km of reach.
        assert_eq!(mount.flight(1_100), Duration::from_millis(4_400));
        assert_eq!(mount.longest_flight(), Duration::from_secs(12));
        let beam = Mount {
            muzzle_speed_cms: 0,
            ..mount
        };
        assert_eq!(beam.flight(1_100), Duration::ZERO);
        assert_eq!(beam.longest_flight(), Duration::ZERO);
    }

    #[test]
    fn the_pulse_is_the_slowest_period_and_a_floor_can_only_slow_it() {
        let mut config = TacsimConfig::default();
        config.arena.tempo.quiet_ms = 700;
        config.arena.tempo.alert_ms = 300;
        config.arena.tempo.engaged_ms = 100;
        let fig = Figures::of(&config);
        assert_eq!(fig.pulse(), Duration::from_millis(700));
        assert_eq!(fig.pulse_under(500), Duration::from_millis(700));
        assert_eq!(fig.pulse_under(5_000), Duration::from_millis(5_000));
    }

    #[test]
    fn a_rounds_limit_is_its_cycle_flight_and_two_pulses_under_the_load_factor() {
        let mut config = TacsimConfig::default();
        config.arena.tempo.quiet_ms = 100;
        config.arena.tempo.alert_ms = 100;
        config.arena.tempo.engaged_ms = 100;
        let fig = Figures::of(&config);
        let cannon = fig.mount("line_tank", "cannon");
        assert_eq!(
            fig.after_one_round(cannon),
            (cannon.cycle() + cannon.longest_flight() + Duration::from_millis(200))
                * crate::LOAD_FACTOR
        );
        config.match_.reclaim_min_ms = 3_000;
        let fig = Figures::of(&config);
        assert_eq!(
            fig.after_reclaim(),
            Duration::from_millis(3_200) * crate::LOAD_FACTOR
        );
    }

    #[test]
    fn a_crossing_is_the_run_up_and_the_distance_at_top_speed() {
        let mut config = TacsimConfig::default();
        config.arena.tempo.quiet_ms = 100;
        config.arena.tempo.alert_ms = 100;
        config.arena.tempo.engaged_ms = 100;
        config.sensors.pass_ms = 1_500;
        let fig = Figures::of(&config);
        let drive = shipped().specs[&CanonicalId::new("tacsim.unit.line_tank").unwrap()]
            .mobility
            .tracked
            .unwrap();
        let top = u64::from(drive.max_speed_cms);
        let want = Duration::from_millis(
            top * 1_000 / u64::from(drive.accel_cms2) + 200 * 100 * 1_000 / top + 200,
        ) * crate::LOAD_FACTOR;
        assert_eq!(fig.crossing("line_tank", 200), want);
        assert!(fig.crossing("line_tank", 400) > fig.crossing("line_tank", 200));
        assert_eq!(fig.sensor_pass_seconds(), 2);
    }

    /// A chosen seed's rolls are the arena's own draws, and the two "whatever
    /// the odds" readings are the resolution table's floor and ceiling.
    #[test]
    fn a_chosen_seed_forces_a_roll() {
        let fig = fig();
        let (floor, ceiling) = fig.chance_bounds();
        assert!(0.0 < floor && floor < ceiling && ceiling < 1.0);
        let lands = fig.seed_where(1, |seed| fig.sure_hit(seed, 1));
        assert!(f64::from(fig.hit_roll(lands, 1)) / 1_000.0 < floor);
        assert_eq!(
            fig.hit_roll(lands, 1),
            u32::from(draw_permille(lands, &DrawLabel::Hit { shot_seq: 1 }))
        );
        let misses = fig.seed_where(1, |seed| fig.sure_miss(seed, 3));
        assert!(f64::from(fig.hit_roll(misses, 3)) / 1_000.0 >= ceiling);
        // Two demands at once: the first round lands and no round of the
        // magazine can jam or damage a part.
        let cannon = fig.mount("line_tank", "cannon");
        let clean = fig.seed_where(1, |seed| {
            fig.sure_hit(seed, 1)
                && (1..=u64::from(cannon.pulls()))
                    .all(|shot| !fig.harms_a_part("line_tank", seed, shot))
        });
        assert!(fig.sure_hit(clean, 1));
        assert!(!fig.harms_a_part("line_tank", clean, 1));
        // And the search starts where it is told to.
        assert!(fig.seed_where(lands, |seed| fig.sure_hit(seed, 1)) == lands);
    }

    #[test]
    #[should_panic(expected = "gives the dice the story asks for")]
    fn a_seed_the_rules_cannot_give_is_refused() {
        let fig = fig();
        let _ = fig.seed_where(1, |seed| fig.sure_hit(seed, 1) && fig.sure_miss(seed, 1));
    }

    #[test]
    fn a_pinned_story_seed_is_the_seed_and_an_unpinned_one_is_fresh() {
        // `STORY_SEED` is not set in a test run of this crate.
        if std::env::var_os("STORY_SEED").is_some() {
            return;
        }
        assert_eq!(story_seed(Some(42)), 42);
        let (one, two) = (story_seed(None), story_seed(None));
        assert_ne!(one, 0);
        assert_eq!(one, two, "one fresh seed a process");
        assert_ne!(fresh_seed(), fresh_seed(), "two draws differ");
    }
}
