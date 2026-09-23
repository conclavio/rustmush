//! # MobilityProfile — the mobility facet of a `TacticalUnit` (design 06 §3.3)
//!
//! `mush-tactics` owns "`MobilityProfile` with registered mode/medium/locomotion
//! ids for multimodal units, **leaving terrain-cost and kinematics maths to the
//! ruleset**." This module is exactly that boundary: the engine provides the
//! vocabulary registry, the multimodal profile, the mode-transition state
//! machine, and capability queries; the ruleset (`tacsim`) provides the
//! concrete mode kinematics/terrain (carried here as an OPAQUE, schema-tagged
//! [`ModeRules`], the mobility analogue of `mush-sim`'s `RulesValue` pose) and
//! all the physics math.
//!
//! ## This cut (M31) is a pure library
//! Types + registry + the transition machine + capability queries, all pure and
//! taking `now` (unix seconds) as a parameter — no `Engine`, no persistence, no
//! command. Attachment to objects (a per-object store, boot lifecycle,
//! `ObjDestroyed` scrub, a staff command) lands with the `TacticalUnit` module
//! that first *reads* a profile; nothing this cut reads an attached one, so a
//! module would be write-only YAGNI. The seam-test [`crate::TacticsStub`] stays
//! green and untouched.
//!
//! ## Determinism (design 06 §6.6)
//! Everything authoritative is integer/fixed-point: kinematics live in the
//! opaque blob (ruleset's problem), the one engine-known magnitude
//! [`MobilityMode::nominal_speed_cms`] is `u32` cm/s, time is `i64` unix seconds,
//! and a transition resolves purely from the stored `ready_at` versus `now`
//! (no float, no wall-clock). Ordered `BTree*` collections keep serialization
//! and any future state hash stable across platforms.
//!
//! ## The mode-transition state machine
//! A profile stores its mode ONCE as a [`ModeState`] — `Settled(id)` or
//! `Switching{from,to,ready_at}` — and resolves the current mode *virtually* off
//! the clock, so no read mutates and correctness never depends on a `settle`
//! call having run. [`MobilityProfile::settle`] is idempotent normalization
//! (fold a *due* `Switching` into `Settled(to)`); call it before serializing so
//! a due-but-unsettled transient never crosses a persistence boundary.
//! Mid-switch, [`MobilityProfile::effective_mode`] is `None` (the strict engine
//! answer: the unit is *between* modes); the raw `from`/`to`/`ready_at` stay
//! exposed via [`MobilityProfile::switch_parts`] and the `mode_*` predicates so a
//! ruleset can implement its own "retain the from-mode until ready" physics
//! without the engine deciding it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// Max canonical length of any mobility id.
pub const MOBILITY_ID_MAX: usize = 64;

/// Canonicalize a mobility id: trim, ASCII-lowercase, then require a non-empty
/// run of `[a-z0-9_.]` with no leading/trailing dot and no empty segment. Both
/// bare (`ground`, `legged`) and namespaced (`tacsim.walk`) forms are accepted;
/// canonicalization guarantees one logical id always serializes to identical
/// bytes, so equal profiles hash equally.
fn canon(s: &str) -> Result<SmolStr, MobilityError> {
    let t = s.trim().to_ascii_lowercase();
    if t.is_empty() {
        return Err(MobilityError::EmptyId);
    }
    if t.len() > MOBILITY_ID_MAX {
        return Err(MobilityError::IdTooLong(t));
    }
    if !t
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
    {
        return Err(MobilityError::MalformedId(t));
    }
    if t.starts_with('.') || t.ends_with('.') || t.split('.').any(str::is_empty) {
        return Err(MobilityError::MalformedId(t));
    }
    Ok(SmolStr::new(t))
}

fn parse_canonical(s: &str) -> Result<SmolStr, MobilityError> {
    let canonical = canon(s)?;
    if canonical.as_str() != s {
        return Err(MobilityError::NonCanonicalId {
            canonical: canonical.to_string(),
        });
    }
    Ok(canonical)
}

/// Define a canonical, `SmolStr`-backed mobility id newtype. Trusted authoring
/// input is canonicalized; persisted/wire input must already be canonical.
macro_rules! mobility_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "SmolStr", into = "SmolStr")]
        pub struct $name(SmolStr);

        impl $name {
            /// Validate and canonicalize (see `canon`).
            ///
            /// # Errors
            /// [`MobilityError::EmptyId`] / [`MobilityError::IdTooLong`] /
            /// [`MobilityError::MalformedId`] if the string is not a valid id.
            pub fn new(s: impl AsRef<str>) -> Result<$name, MobilityError> {
                Ok($name(canon(s.as_ref())?))
            }
            /// The canonical id as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<SmolStr> for $name {
            type Error = MobilityError;
            fn try_from(s: SmolStr) -> Result<$name, MobilityError> {
                Ok($name(parse_canonical(s.as_str())?))
            }
        }
        impl From<$name> for SmolStr {
            fn from(v: $name) -> SmolStr {
                v.0
            }
        }
        impl std::str::FromStr for $name {
            type Err = MobilityError;
            fn from_str(s: &str) -> Result<$name, MobilityError> {
                $name::new(s)
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }
    };
}

mobility_id! {
    /// A physical medium a mode operates in (`ground`, `air`, `water`, `space`).
    /// A mode may list several (an amphibian crosses `ground` *and* `water`).
    MediumId
}
mobility_id! {
    /// How a mode locomotes (`legged`, `wheeled`, `tracked`, `hover`, `rotor`,
    /// `fixed_wing`, …). A coarse physical category, not a game rule.
    LocomotionId
}
mobility_id! {
    /// An open-ended capability tag on a mode (`amphibious`, `jump`, `vtol`, …).
    MobilityTag
}
mobility_id! {
    /// The id of a mode within a profile (`ground.tracked`, `air.rotor`, …).
    /// Unique per profile; used to select the active mode and target a switch.
    MobilityModeId
}

/// Opaque, exactly schema-referenced ruleset physics for a mode — kinematics,
/// terrain response, whatever the setting needs. This alias deliberately uses
/// the shared mechanics value: mobility data must carry the same owner, kind,
/// exact version, and digest as every other rules-bearing payload.
pub type ModeRules = mush_core::RulesValue;

/// One self-contained mode a unit can be in (design: modes are inline on the
/// profile, not registry templates — the field that varies most per unit is the
/// opaque per-unit kinematics, so a template + override side-channel earns
/// nothing this cut). Its atoms (`mediums`/`locomotion`/`tags`) must be declared
/// in the [`MobilityRegistry`] the profile is validated against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobilityMode {
    /// The mode's id (unique within its profile).
    pub id: MobilityModeId,
    /// The media it traverses (non-empty; several = multi-medium, e.g. amphibious).
    pub mediums: BTreeSet<MediumId>,
    /// How it locomotes.
    pub locomotion: LocomotionId,
    /// Capability tags.
    #[serde(default)]
    pub tags: BTreeSet<MobilityTag>,
    /// Opaque ruleset kinematics/terrain physics.
    pub rules: ModeRules,
    /// The ONE engine-known coarse magnitude: nominal speed in cm/s. Reserved
    /// for the deferred route/`MobilityJob` layer (doc 07 §9) to size a
    /// `travel_s` leg without the ruleset executor. `0` = unspecified.
    #[serde(default)]
    pub nominal_speed_cms: u32,
    /// Reserved: canonical seconds to switch INTO this mode (a transform/spool
    /// time), for the future control path. `0` = instant. The engine does not
    /// apply it this cut (`begin_switch` takes an explicit duration).
    #[serde(default)]
    pub transition_s: u32,
}

impl MobilityMode {
    /// A mode with the given media and locomotion, no tags, zero speeds.
    #[must_use]
    pub fn new(
        id: MobilityModeId,
        mediums: impl IntoIterator<Item = MediumId>,
        locomotion: LocomotionId,
        rules: ModeRules,
    ) -> MobilityMode {
        MobilityMode {
            id,
            mediums: mediums.into_iter().collect(),
            locomotion,
            tags: BTreeSet::new(),
            rules,
            nominal_speed_cms: 0,
            transition_s: 0,
        }
    }
    /// Builder: set the capability tags.
    #[must_use]
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = MobilityTag>) -> MobilityMode {
        self.tags = tags.into_iter().collect();
        self
    }
    /// Builder: set the nominal speed (cm/s) and the transition time (seconds).
    #[must_use]
    pub fn with_speeds(mut self, nominal_speed_cms: u32, transition_s: u32) -> MobilityMode {
        self.nominal_speed_cms = nominal_speed_cms;
        self.transition_s = transition_s;
        self
    }
}

// ----------------------------------------------------------------------
// The vocabulary: a builder composed at wiring time, frozen for runtime
// ----------------------------------------------------------------------

/// A mutable vocabulary of declared atoms (media/locomotions/tags). Composed at
/// MODULE/WIRING construction — the engine seeds a generic [`base`](Self::base)
/// and a ruleset `seed`s its own additions — then [`freeze`](Self::freeze)d into
/// the read-only [`MobilityRegistry`]. (A `reg.state()` slot cannot serve this:
/// slots materialize only *after* every module registers, so there is no
/// cross-module runtime contribution window; construction-time composition is.)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MobilityVocabulary {
    mediums: BTreeSet<MediumId>,
    locomotions: BTreeSet<LocomotionId>,
    tags: BTreeSet<MobilityTag>,
}

impl MobilityVocabulary {
    /// An empty vocabulary.
    #[must_use]
    pub fn new() -> MobilityVocabulary {
        MobilityVocabulary::default()
    }

    /// The engine's generic base vocabulary: the physical categories any ruleset
    /// shares. Deliberately minimal — game-specific media/locomotions/tags are
    /// content a ruleset declares on top.
    #[must_use]
    pub fn base() -> MobilityVocabulary {
        // S5 (doc 14 §5.1): the atoms are the atlas's — one list, the
        // locomotion table keyed by it — with `biped` the pedestrian default.
        let mut v = MobilityVocabulary::new();
        for m in mush_atlas::physics::BASE_MEDIA {
            v.declare_medium(MediumId::new(m).expect("base medium id"));
        }
        for l in mush_atlas::physics::BASE_LOCOMOTIONS {
            v.declare_locomotion(LocomotionId::new(l).expect("base locomotion id"));
        }
        for t in mush_atlas::physics::BASE_TAGS {
            v.declare_tag(MobilityTag::new(t).expect("base tag id"));
        }
        v
    }

    /// Declare a medium (idempotent).
    pub fn declare_medium(&mut self, m: MediumId) -> &mut MobilityVocabulary {
        self.mediums.insert(m);
        self
    }
    /// Declare a locomotion (idempotent).
    pub fn declare_locomotion(&mut self, l: LocomotionId) -> &mut MobilityVocabulary {
        self.locomotions.insert(l);
        self
    }
    /// Declare a capability tag (idempotent).
    pub fn declare_tag(&mut self, t: MobilityTag) -> &mut MobilityVocabulary {
        self.tags.insert(t);
        self
    }

    /// Freeze into the read-only registry used to validate profiles.
    #[must_use]
    pub fn freeze(self) -> MobilityRegistry {
        MobilityRegistry {
            mediums: self.mediums,
            locomotions: self.locomotions,
            tags: self.tags,
        }
    }
}

/// The frozen atom vocabulary a [`MobilityProfile`] is validated against.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MobilityRegistry {
    mediums: BTreeSet<MediumId>,
    locomotions: BTreeSet<LocomotionId>,
    tags: BTreeSet<MobilityTag>,
}

impl MobilityRegistry {
    /// Whether `m` is a declared medium.
    #[must_use]
    pub fn has_medium(&self, m: &MediumId) -> bool {
        self.mediums.contains(m)
    }
    /// Whether `l` is a declared locomotion.
    #[must_use]
    pub fn has_locomotion(&self, l: &LocomotionId) -> bool {
        self.locomotions.contains(l)
    }
    /// Whether `t` is a declared tag.
    #[must_use]
    pub fn has_tag(&self, t: &MobilityTag) -> bool {
        self.tags.contains(t)
    }

    /// Validate a mode's atoms are all declared and it names at least one medium.
    ///
    /// # Errors
    /// [`MobilityError::NoMediums`] / `Undeclared*` for the first offending atom.
    pub fn validate_mode(&self, m: &MobilityMode) -> Result<(), MobilityError> {
        if m.mediums.is_empty() {
            return Err(MobilityError::NoMediums(m.id.clone()));
        }
        for med in &m.mediums {
            if !self.has_medium(med) {
                return Err(MobilityError::UndeclaredMedium(med.clone()));
            }
        }
        if !self.has_locomotion(&m.locomotion) {
            return Err(MobilityError::UndeclaredLocomotion(m.locomotion.clone()));
        }
        for t in &m.tags {
            if !self.has_tag(t) {
                return Err(MobilityError::UndeclaredTag(t.clone()));
            }
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------
// The mode-transition state machine
// ----------------------------------------------------------------------

/// A profile's mode, stored ONCE. The current mode resolves virtually off the
/// clock; see [`MobilityProfile::effective_mode`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ModeState {
    /// Settled in one mode.
    Settled(MobilityModeId),
    /// Switching from `from` to `to`, complete at `ready_at` (unix seconds).
    Switching {
        /// The last settled mode (its capabilities may or may not persist mid-
        /// switch — that is a ruleset call, not the engine's).
        from: MobilityModeId,
        /// The target mode, effective once `now >= ready_at`.
        to: MobilityModeId,
        /// When the switch completes (unix seconds).
        ready_at: i64,
    },
}

/// A unit's multimodal mobility: its available modes and current [`ModeState`].
/// Fields are private; build via [`new`](Self::new) or `serde` (which enforces
/// the intrinsic invariants), never by hand.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ProfileRepr", into = "ProfileRepr")]
pub struct MobilityProfile {
    modes: Vec<MobilityMode>,
    mode: ModeState,
}

/// The serialized shape of a [`MobilityProfile`], so `serde` re-checks the
/// intrinsic invariants (non-empty, unique ids, state ids ∈ modes) on load and a
/// structurally broken document fails to deserialize. Registry (extrinsic)
/// validity is re-checked separately by the future boot pass, since a ruleset
/// may change its vocabulary between boots.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRepr {
    modes: Vec<MobilityMode>,
    mode: ModeState,
}

impl From<MobilityProfile> for ProfileRepr {
    fn from(p: MobilityProfile) -> ProfileRepr {
        ProfileRepr {
            modes: p.modes,
            mode: p.mode,
        }
    }
}
impl TryFrom<ProfileRepr> for MobilityProfile {
    type Error = MobilityError;
    fn try_from(r: ProfileRepr) -> Result<MobilityProfile, MobilityError> {
        MobilityProfile::from_parts(r.modes, r.mode)
    }
}

impl MobilityProfile {
    /// Build a profile from its available modes and initial active mode,
    /// validating every mode against `reg` (extrinsic) and the structural
    /// invariants (intrinsic).
    ///
    /// # Errors
    /// A `Undeclared*`/`NoMediums` for an invalid mode, or `EmptyProfile` /
    /// `DuplicateMode` / `UnknownMode` for a structural violation.
    pub fn new(
        modes: Vec<MobilityMode>,
        active: MobilityModeId,
        reg: &MobilityRegistry,
    ) -> Result<MobilityProfile, MobilityError> {
        for m in &modes {
            reg.validate_mode(m)?;
        }
        MobilityProfile::from_parts(modes, ModeState::Settled(active))
    }

    /// The intrinsic-invariant constructor shared by `new` and deserialize:
    /// non-empty, unique mode ids, and every id named by `mode` present.
    pub(crate) fn from_parts(
        modes: Vec<MobilityMode>,
        mode: ModeState,
    ) -> Result<MobilityProfile, MobilityError> {
        if modes.is_empty() {
            return Err(MobilityError::EmptyProfile);
        }
        let mut seen: BTreeSet<&MobilityModeId> = BTreeSet::new();
        for m in &modes {
            if !seen.insert(&m.id) {
                return Err(MobilityError::DuplicateMode(m.id.clone()));
            }
            // Registry-INDEPENDENT structural invariant: a mode with no medium
            // is malformed regardless of vocabulary, so it must be rejected on
            // BOTH construction and deserialize (not only in validate_mode,
            // which new() runs but serde does not).
            if m.mediums.is_empty() {
                return Err(MobilityError::NoMediums(m.id.clone()));
            }
        }
        let known = |id: &MobilityModeId| modes.iter().any(|m| &m.id == id);
        match &mode {
            ModeState::Settled(a) => {
                if !known(a) {
                    return Err(MobilityError::UnknownMode(a.clone()));
                }
            }
            ModeState::Switching { from, to, .. } => {
                if !known(from) {
                    return Err(MobilityError::UnknownMode(from.clone()));
                }
                if !known(to) {
                    return Err(MobilityError::UnknownMode(to.clone()));
                }
            }
        }
        Ok(MobilityProfile { modes, mode })
    }

    /// The available modes.
    #[must_use]
    pub fn modes(&self) -> &[MobilityMode] {
        &self.modes
    }
    /// The raw stored mode state.
    #[must_use]
    pub fn mode_state(&self) -> &ModeState {
        &self.mode
    }
    /// The mode with id `id`, if available.
    #[must_use]
    pub fn mode_by_id(&self, id: &MobilityModeId) -> Option<&MobilityMode> {
        self.modes.iter().find(|m| &m.id == id)
    }

    /// The id of the mode in effect at `now`: the settled mode, or — mid-switch —
    /// `None` before `ready_at` and the target once `now >= ready_at`. `None`
    /// mid-switch is the strict engine answer (the unit is *between* modes) and
    /// is the interruptibility signal; a ruleset that wants to project the
    /// from-mode's capabilities during the switch reads [`switch_parts`](Self::switch_parts).
    #[must_use]
    pub fn effective_mode(&self, now: i64) -> Option<&MobilityModeId> {
        match &self.mode {
            ModeState::Settled(m) => Some(m),
            ModeState::Switching { to, ready_at, .. } => (now >= *ready_at).then_some(to),
        }
    }
    /// The full [`MobilityMode`] in effect at `now` (see [`effective_mode`](Self::effective_mode)).
    #[must_use]
    pub fn effective(&self, now: i64) -> Option<&MobilityMode> {
        self.effective_mode(now).and_then(|id| self.mode_by_id(id))
    }
    /// Whether a switch is in progress (not yet complete) at `now`.
    #[must_use]
    pub fn is_switching(&self, now: i64) -> bool {
        matches!(&self.mode, ModeState::Switching { ready_at, .. } if now < *ready_at)
    }
    /// The raw `(from, to, ready_at)` of an in-progress or due-but-unsettled
    /// switch — the escape hatch for a ruleset to apply its own mid-switch
    /// capability policy. `None` when settled.
    #[must_use]
    pub fn switch_parts(&self) -> Option<(&MobilityModeId, &MobilityModeId, i64)> {
        match &self.mode {
            ModeState::Switching { from, to, ready_at } => Some((from, to, *ready_at)),
            ModeState::Settled(_) => None,
        }
    }

    /// Idempotent normalization: fold a *due* switch (`now >= ready_at`) into
    /// `Settled(to)`. No read depends on this having run; call it before
    /// serializing so a due-but-unsettled transient never persists. A settled
    /// profile can never un-settle, so this is safe even if `now` moves backward.
    pub fn settle(&mut self, now: i64) {
        if let ModeState::Switching { to, ready_at, .. } = &self.mode
            && now >= *ready_at
        {
            self.mode = ModeState::Settled(to.clone());
        }
    }

    /// Begin (or supersede) a switch to `to`, completing `dur_s` seconds from
    /// `now`. Normalizes first, so `from` is always the last *committed* mode:
    /// re-issuing mid-switch supersedes the target, and switching back to `from`
    /// mid-switch is a clean instant abort. `dur_s == 0` is an instant switch.
    ///
    /// # Errors
    /// [`MobilityError::ModeNotAvailable`] if `to` is not one of this profile's
    /// modes; [`MobilityError::AlreadyInMode`] if already settled in `to`.
    pub fn begin_switch(
        &mut self,
        to: &MobilityModeId,
        now: i64,
        dur_s: u32,
    ) -> Result<(), MobilityError> {
        if self.mode_by_id(to).is_none() {
            return Err(MobilityError::ModeNotAvailable(to.clone()));
        }
        self.settle(now);
        let from = match &self.mode {
            ModeState::Settled(m) => m.clone(),
            ModeState::Switching { from, .. } => from.clone(),
        };
        if &from == to {
            // Settled in `to` already → no-op error; mid-switch back to `from` →
            // clean abort. Either way the resting state is Settled(from == to).
            if matches!(self.mode, ModeState::Settled(_)) {
                return Err(MobilityError::AlreadyInMode(to.clone()));
            }
            self.mode = ModeState::Settled(from);
            return Ok(());
        }
        let ready_at = now.saturating_add(i64::from(dur_s));
        self.mode = if ready_at <= now {
            ModeState::Settled(to.clone())
        } else {
            ModeState::Switching {
                from,
                to: to.clone(),
                ready_at,
            }
        };
        Ok(())
    }

    // --- capability queries on the EFFECTIVE mode (typed, never a bare id) ---

    /// Whether the effective mode at `now` carries `tag`. `false` mid-switch.
    #[must_use]
    pub fn can_tag(&self, tag: &MobilityTag, now: i64) -> bool {
        self.effective(now).is_some_and(|m| m.tags.contains(tag))
    }
    /// Whether the effective mode at `now` traverses `medium`. `false` mid-switch.
    #[must_use]
    pub fn in_medium(&self, medium: &MediumId, now: i64) -> bool {
        self.effective(now)
            .is_some_and(|m| m.mediums.contains(medium))
    }
    /// Whether the effective mode at `now` uses `loco`. `false` mid-switch.
    #[must_use]
    pub fn is_locomotion(&self, loco: &LocomotionId, now: i64) -> bool {
        self.effective(now).is_some_and(|m| &m.locomotion == loco)
    }

    // --- mode-explicit predicates (ignore the transition; for ruleset policy) ---

    /// Whether the named mode carries `tag` (regardless of the current switch).
    #[must_use]
    pub fn mode_has_tag(&self, mode: &MobilityModeId, tag: &MobilityTag) -> bool {
        self.mode_by_id(mode).is_some_and(|m| m.tags.contains(tag))
    }
    /// Whether the named mode traverses `medium` (regardless of the current switch).
    #[must_use]
    pub fn mode_in_medium(&self, mode: &MobilityModeId, medium: &MediumId) -> bool {
        self.mode_by_id(mode)
            .is_some_and(|m| m.mediums.contains(medium))
    }
}

/// Why a mobility operation was refused. Carries the offending id; `Display` is
/// suitable for a command layer to surface (mirrors `mush-sim`'s `SpatialError`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MobilityError {
    /// An id string was empty after trimming.
    #[error("mobility id is empty")]
    EmptyId,
    /// An id exceeded [`MOBILITY_ID_MAX`].
    #[error("mobility id {0:?} is too long (max {MOBILITY_ID_MAX})")]
    IdTooLong(String),
    /// An id had characters outside `[a-z0-9_.]` or a bad dot segment.
    #[error(
        "mobility id {0:?} must be lowercase [a-z0-9_.] with no leading/trailing/empty dot segment"
    )]
    MalformedId(String),
    /// Persisted/wire input was valid only after normalization. Reject it so
    /// distinct byte strings cannot collapse to one authoritative identity.
    #[error("mobility id is noncanonical; use {canonical:?}")]
    NonCanonicalId { canonical: String },
    /// A profile had no modes.
    #[error("a mobility profile must have at least one mode")]
    EmptyProfile,
    /// Two modes shared an id.
    #[error("duplicate mode id {0}")]
    DuplicateMode(MobilityModeId),
    /// A `ModeState` named a mode the profile does not have.
    #[error("mode {0} is not in this profile")]
    UnknownMode(MobilityModeId),
    /// `begin_switch` targeted a mode the profile does not have.
    #[error("cannot switch to {0}: not an available mode")]
    ModeNotAvailable(MobilityModeId),
    /// `begin_switch` targeted the already-settled mode.
    #[error("already in mode {0}")]
    AlreadyInMode(MobilityModeId),
    /// A mode named no medium.
    #[error("mode {0} must name at least one medium")]
    NoMediums(MobilityModeId),
    /// A mode referenced an undeclared medium.
    #[error("undeclared medium {0}")]
    UndeclaredMedium(MediumId),
    /// A mode referenced an undeclared locomotion.
    #[error("undeclared locomotion {0}")]
    UndeclaredLocomotion(LocomotionId),
    /// A mode referenced an undeclared tag.
    #[error("undeclared tag {0}")]
    UndeclaredTag(MobilityTag),
}
