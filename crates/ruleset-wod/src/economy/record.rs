//! The E5c records (08d §5.2, §5.3): the spending profile, the career, the four
//! stances, front references, and the one prompt.
//!
//! Two rules shape every type here.
//!
//! **The ruleset interprets dots; housing does not** (D-E5-5). A
//! [`SpendingProfile`] is derived from `backgrounds:Resources` and the character's
//! stance, and what crosses into `mush-housing` is a [`mush_housing::SupportPlan`] —
//! an opaque `0..=5` and a list of attribution labels. Nothing in this file is
//! readable by housing and nothing in housing reads a sheet.
//!
//! **The economy never writes a sheet dot** (D-E5-3). `resources` here is a *cache
//! of what the sheet said at the last evaluation*, refreshed from the sheet every
//! period and never written back. A permanent Resources change goes through
//! `+xp/spend` like every other trait.

use std::collections::BTreeMap;

use mush_core::ObjId;
use mush_faction::{FactionId, FrontId};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::economy::config::Engagement;

/// The ceiling on a support level and a Resources rating (§9's `0..=5` band).
pub const MAX_LEVEL: u8 = 5;
/// Strain's ceiling (§9).
pub const STRAIN_MAX: u8 = 10;
/// How often `Coast` may produce an opportunity (§9).
pub const COAST_OPPORTUNITY_PERIODS: u64 = 6;
/// The per-faction cap on **currently open** off-screen operations (§9).
pub const MAX_OFFSCREEN_OPS_PER_FACTION: u32 = 8;
/// A prompt's default life, in cadence periods (§9).
pub const PROMPT_EXPIRY_PERIODS: u32 = 2;
/// The bounded per-profile history tail (§9's "history … cap 32").
pub const MAX_PROFILE_HISTORY: usize = 32;
/// The most attribution labels one plan carries — `mush_housing::MAX_PLAN_SOURCES`,
/// restated so a profile refuses before housing has to.
pub const MAX_SOURCES: usize = 8;
/// The largest free-text field any of these records stores (the `mush-economy`
/// bounded-string discipline E5a adopted).
pub const MAX_TEXT_BYTES: usize = 64;

/// A pointer at the faction fiction a piece of progress came from (owner answer 20).
///
/// This is the **pre-C6 shape** (08d §7 item 3): once doc 09's C6 causal chain lands,
/// front attribution nests inside the kernel's own chain and this becomes a
/// projection of it. Until then it is three ids, and the render tolerates every one
/// of them having gone away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FrontRef {
    /// Whose front.
    pub faction: FactionId,
    /// Which front.
    pub front: FrontId,
    /// The operation within it, when there is one.
    #[serde(default)]
    pub operation: Option<u64>,
}

impl FrontRef {
    /// The canon `source` this front attributes to: `front:{faction}:{front}`
    /// (§5.3). E4c matches a canon attestor on **kind and source**, so this string
    /// is the whole point — a `scene.concluded` with any faction member is never an
    /// attribution.
    #[must_use]
    pub fn source(&self) -> String {
        format!("front:{}:{}", self.faction.0, self.front.0)
    }

    /// The raw-id render an **orphaned** front gets: never a failed read (§5.3).
    #[must_use]
    pub fn raw(&self) -> String {
        match self.operation {
            Some(op) => format!("front #{}/#{} op #{op}", self.faction.0, self.front.0),
            None => format!("front #{}/#{}", self.faction.0, self.front.0),
        }
    }
}

/// The four deterministic stances (owner answer 11, D-E5-6).
///
/// A stance alters **pressure and opportunity**, never income: none of the four
/// changes what an exact-mode obligation pays, and none of them writes a sheet dot.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Stance {
    /// Stable, low pressure, few opportunities.
    #[default]
    Coast,
    /// More time demand and strain; advancement prompts; **no automatic dot gain**.
    Advance,
    /// Temporary capacity at increased exposure and risk.
    Exploit,
    /// Reduced visibility at reduced opportunity and capacity.
    Conceal,
}

impl Stance {
    /// The command word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Stance::Coast => "coast",
            Stance::Advance => "advance",
            Stance::Exploit => "exploit",
            Stance::Conceal => "conceal",
        }
    }

    /// Parse a `+career/stance` word.
    #[must_use]
    pub fn parse(s: &str) -> Option<Stance> {
        match s.trim().to_ascii_lowercase().as_str() {
            "coast" => Some(Stance::Coast),
            "advance" => Some(Stance::Advance),
            "exploit" => Some(Stance::Exploit),
            "conceal" => Some(Stance::Conceal),
            _ => None,
        }
    }

    /// The support-level modifier this stance applies **this period** (§5.2's table).
    ///
    /// `Exploit`'s `+1` is explicitly temporary: it is re-derived every period from
    /// the sheet plus this number, so dropping the stance drops the capacity with it,
    /// and nothing anywhere writes the dot.
    #[must_use]
    pub fn level_delta(self) -> i16 {
        match self {
            Stance::Coast | Stance::Advance => 0,
            Stance::Exploit => 1,
            Stance::Conceal => -1,
        }
    }

    /// The strain delta this stance applies per period (§5.2's table).
    ///
    /// `Coast`'s `-1` **is** §5.2's "decays 1/period"; the two are one number, not
    /// two that stack.
    #[must_use]
    pub fn strain_delta(self) -> i16 {
        match self {
            Stance::Coast => -1,
            Stance::Advance | Stance::Exploit => 1,
            Stance::Conceal => 0,
        }
    }

    /// Whether this stance emits a front-attributed canon evidence event **every**
    /// period (§5.2: `Exploit` high, `Conceal` none).
    #[must_use]
    pub fn emits_exposure(self) -> bool {
        matches!(self, Stance::Exploit)
    }

    /// The exposure word `+career` prints.
    #[must_use]
    pub fn exposure_word(self) -> &'static str {
        match self {
            Stance::Coast => "none",
            Stance::Advance => "low",
            Stance::Exploit => "high",
            Stance::Conceal => "reduced",
        }
    }

    /// The prompt kind this stance offers, if any, on a period it may offer one.
    #[must_use]
    pub fn offers(self) -> Option<PromptShape> {
        match self {
            Stance::Coast | Stance::Exploit => Some(PromptShape::Opportunity),
            Stance::Advance => Some(PromptShape::Advancement),
            Stance::Conceal => None,
        }
    }
}

/// Which of the two front-attributed prompt shapes a stance offers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PromptShape {
    /// A piece of off-screen work.
    Opportunity,
    /// A step up, with no automatic dot gain.
    Advancement,
}

/// One career (§5.2). Housing sees only its opaque `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CareerProfile {
    /// A free-text kind (`"detective"`, `"fixer"`), ≤ [`MAX_TEXT_BYTES`].
    pub kind: SmolStr,
    /// An optional employer principal (owner answer 12).
    #[serde(default)]
    pub employer: Option<PrincipalId>,
    /// The stance **in force this period**.
    #[serde(default)]
    pub stance: Stance,
    /// A stance chosen since the last boundary, applied at the **next** one
    /// (D-E5-6). Recorded here rather than swapped immediately because "a flip to
    /// `Exploit` seconds before assessment changes nothing" is a rule, and a rule
    /// needs somewhere to hold the pending answer.
    #[serde(default)]
    pub pending_stance: Option<Stance>,
    /// Flavour, `0..=5`, shown by `+career`.
    #[serde(default)]
    pub time_demand: u8,
    /// Flavour, `0..=5`.
    #[serde(default)]
    pub visibility: u8,
    /// Flavour, `0..=5`.
    #[serde(default)]
    pub risk: u8,
    /// The front this career's progress is attributed to (owner answer 20). Without
    /// one it generates **nothing**: every opportunity and every exposure event must
    /// cite a specific front, so a career with no front is quiet by construction.
    #[serde(default)]
    pub front: Option<FrontRef>,
}

impl CareerProfile {
    /// A career of `kind`, everything else at its default.
    #[must_use]
    pub fn new(kind: &str) -> CareerProfile {
        CareerProfile {
            kind: SmolStr::new(clip(kind)),
            employer: None,
            stance: Stance::Coast,
            pending_stance: None,
            time_demand: 0,
            visibility: 0,
            risk: 0,
            front: None,
        }
    }
}

/// One reason a character can afford their life (§5.2; housing's
/// [`mush_housing::SupportSource`] is the attribution-only projection of this).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportSource {
    /// A job of some kind, with its whole profile.
    Career(Box<CareerProfile>),
    /// Somebody funds them.
    Patron {
        /// Who.
        who: PrincipalId,
    },
    /// Old money.
    Inheritance,
    /// Income nobody declares.
    Criminal,
    /// An organization's support.
    Domain {
        /// Which org.
        org: OrgId,
    },
    /// **Reserved** (D-E5-10). Dependents are out of E5; nothing constructs this.
    Dependent,
}

impl SupportSource {
    /// The `+career` / `+resources` word.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            SupportSource::Career(c) => format!("career ({})", c.kind),
            SupportSource::Patron { .. } => "patron".to_string(),
            SupportSource::Inheritance => "inheritance".to_string(),
            SupportSource::Criminal => "criminal income".to_string(),
            SupportSource::Domain { .. } => "domain support".to_string(),
            SupportSource::Dependent => "dependent".to_string(),
        }
    }

    /// The command word `+career/source add|remove` takes.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            SupportSource::Career(_) => "career",
            SupportSource::Patron { .. } => "patron",
            SupportSource::Inheritance => "inheritance",
            SupportSource::Criminal => "criminal",
            SupportSource::Domain { .. } => "domain",
            SupportSource::Dependent => "dependent",
        }
    }

    /// The attribution-only projection housing stores on a [`mush_housing::SupportPlan`].
    ///
    /// Deliberately lossy: a career crosses as its opaque `kind` and nothing else
    /// (D-E5-5). The employer, the stance, the front and the risk numbers are the
    /// ruleset's, and housing has no business reasoning about any of them.
    #[must_use]
    pub fn to_housing(&self) -> mush_housing::SupportSource {
        match self {
            SupportSource::Career(c) => {
                mush_housing::SupportSource::Career { kind: c.kind.clone() }
            }
            SupportSource::Patron { who } => {
                mush_housing::SupportSource::Patron { who: who.clone() }
            }
            SupportSource::Inheritance => mush_housing::SupportSource::Inheritance,
            SupportSource::Criminal => mush_housing::SupportSource::Criminal,
            SupportSource::Domain { org } => mush_housing::SupportSource::Domain { org: *org },
            SupportSource::Dependent => mush_housing::SupportSource::Dependent,
        }
    }
}

/// Haven dots, interpreted (D-E5-7, owner answer 15).
///
/// **Recorded, no mechanics.** Haven never selects a property, never discounts rent,
/// and never becomes a [`mush_housing::PropertyTier`]: the whole decision is that
/// exceptional qualities are description, and housing tier is physical class.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HavenQualities {
    /// The Haven background rating the qualities were read from.
    pub dots: u8,
    /// One short phrase per quality, ≤ [`MAX_SOURCES`] of them.
    #[serde(default)]
    pub qualities: Vec<SmolStr>,
}

impl HavenQualities {
    /// The qualities `dots` buys — a fixed, deterministic ladder, so two characters
    /// with the same rating read the same and nobody rolls anything.
    #[must_use]
    pub fn from_dots(dots: u8) -> HavenQualities {
        const LADDER: [&str; MAX_LEVEL as usize] =
            ["secure", "private", "warded", "hidden", "storied"];
        let n = usize::from(dots.min(MAX_LEVEL));
        HavenQualities {
            dots: dots.min(MAX_LEVEL),
            qualities: LADDER.iter().take(n).map(|q| SmolStr::new_static(q)).collect(),
        }
    }
}

/// What a prompt is about (§5.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptKind {
    /// A piece of off-screen work on a named front.
    Opportunity {
        /// Whose, and which.
        front: FrontRef,
    },
    /// Somebody's home is behind. **Preempts** the two front-attributed kinds and is
    /// never preempted by them (D-E5-8).
    HousingPressure {
        /// Which tenancy.
        tenancy: mush_housing::TenancyId,
    },
    /// Strain is at its ceiling.
    StrainRelief,
    /// A step up on a named front.
    Advancement {
        /// Whose, and which.
        front: FrontRef,
    },
}

impl PromptKind {
    /// The one-word render.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            PromptKind::Opportunity { .. } => "opportunity",
            PromptKind::HousingPressure { .. } => "housing",
            PromptKind::StrainRelief => "strain",
            PromptKind::Advancement { .. } => "advancement",
        }
    }

    /// The front this prompt cites, when it cites one. Exactly the two kinds that
    /// count against the per-faction pool.
    #[must_use]
    pub fn front(&self) -> Option<FrontRef> {
        match self {
            PromptKind::Opportunity { front } | PromptKind::Advancement { front } => Some(*front),
            PromptKind::HousingPressure { .. } | PromptKind::StrainRelief => None,
        }
    }

    /// Whether a `HousingPressure` prompt may displace this one (D-E5-8).
    #[must_use]
    pub fn displaceable_by_housing(&self) -> bool {
        matches!(
            self,
            PromptKind::Opportunity { .. } | PromptKind::Advancement { .. }
        )
    }
}

/// What happens when nobody answers (§5.3, owner answer 19).
///
/// **Never a failed homework assignment.** Neither variant adds strain, lowers
/// support, moves money, or notifies as a failure.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptDefault {
    /// Nothing changes; one history note.
    #[default]
    StatusQuo,
    /// The story window stays open for the ST; nothing else changes.
    PendingWindow,
}

impl PromptDefault {
    /// The render word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PromptDefault::StatusQuo => "status quo",
            PromptDefault::PendingWindow => "leave it with staff",
        }
    }
}

/// What choosing one option does. A closed, deterministic alphabet: a prompt is a
/// question about pressure, and pressure is the only thing an answer moves.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptOutcome {
    /// Nothing changes but the note.
    StatusQuo,
    /// Take the strain on (`+n`, clamped at [`STRAIN_MAX`]).
    Strain(u8),
    /// Let some of it go (`-n`, clamped at 0).
    Relief(u8),
}

/// One answer a player may give.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptOption {
    /// What `+prompt <key>` takes.
    pub key: SmolStr,
    /// The sentence shown.
    pub label: SmolStr,
    /// What it does.
    pub outcome: PromptOutcome,
}

/// **The one unresolved personal economic prompt** (owner answer 18, D-E5-8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    /// Identity, from the store's allocator.
    pub id: u64,
    /// What it is about.
    pub kind: PromptKind,
    /// The answers, ≤ 4.
    pub options: Vec<PromptOption>,
    /// What happens if nobody answers.
    pub default: PromptDefault,
    /// The cadence **position** it opened at (the stamp `+prompt` prints).
    pub opened_at: i64,
    /// The monotonic period ordinal it opened in — what expiry is measured in, so a
    /// re-pointed clock cannot make a prompt immortal or instantly stale.
    pub opened_period: u64,
    /// The world-IC stamp it opened at (K4a's versioned timeline), recorded whole so
    /// its generation travels with it and it can never be compared across a rate
    /// change by accident.
    #[serde(default)]
    pub opened_ic: Option<mush_core::clock::ClockStamp>,
    /// How many periods it lives.
    pub expires_after_periods: u32,
}

impl Prompt {
    /// Whether `period` is past this prompt's life.
    ///
    /// A stamp from a **future** ordinal is stale, not fresh (E5c fold T3). The
    /// subtraction saturates, so a prompt opened at ordinal 5 against a cursor that
    /// was re-seeded to 0 — a boot re-derivation, a restored backup — would otherwise
    /// read as not-expired for ordinals `0..=6`: seven periods of life instead of
    /// two, holding the character's one attention slot the whole time. The clock
    /// moving backwards under a stamp is the same class of event
    /// `mush_housing::assess` decides explicitly, and this is the decision here.
    #[must_use]
    pub fn expired(&self, period: u64) -> bool {
        if self.opened_period > period {
            return true;
        }
        period.saturating_sub(self.opened_period) >= u64::from(self.expires_after_periods)
    }

    /// The option `key` names, case-insensitively.
    #[must_use]
    pub fn option(&self, key: &str) -> Option<&PromptOption> {
        self.options.iter().find(|o| o.key.eq_ignore_ascii_case(key.trim()))
    }
}

/// One line of a profile's bounded history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileEvent {
    /// The cadence position.
    pub at: i64,
    /// What happened, ≤ [`MAX_TEXT_BYTES`] × 4.
    pub what: SmolStr,
}

/// One character's WoD economic standing (§5.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendingProfile {
    /// Whose.
    pub obj: ObjId,
    /// `backgrounds:Resources` **as of the last evaluation** — a cache for display,
    /// re-read from the sheet every period and never written back (D-E5-3).
    #[serde(default)]
    pub resources: u8,
    /// Why they can afford their life.
    #[serde(default)]
    pub sources: Vec<SupportSource>,
    /// The derived level: `resources ± stance`, clamped `0..=`[`MAX_LEVEL`].
    #[serde(default)]
    pub level: u8,
    /// `0..=`[`STRAIN_MAX`].
    #[serde(default)]
    pub strain: u8,
    /// Player-set; defaults from `[wod.economy].default_engagement`.
    #[serde(default)]
    pub engagement: Engagement,
    /// Haven dots interpreted (D-E5-7) — recorded, no mechanics.
    #[serde(default)]
    pub haven: Option<HavenQualities>,
    /// **The** one unresolved prompt.
    #[serde(default)]
    pub prompt: Option<Prompt>,
    /// Exact mode: this character's `wod.usd` account.
    #[serde(default)]
    pub account: Option<mush_economy::AccountId>,
    /// Exact mode: the faucet-funded income obligation this profile maintains.
    ///
    /// A **cache, not the name**: the obligation identifies itself by `cause`
    /// (`wod.income`), `minted_by` (`system:wod.income`) and `credit_account`, which
    /// is what lets a boot adopt a line whose pointer was lost (E5c fold M3).
    #[serde(default)]
    pub income: Option<mush_economy::ObligationId>,
    /// Exact mode: the Resources dot in force when a **person** cancelled this
    /// character's income line (E5c fold M4).
    ///
    /// `@obligation/cancel` on an income line is a decision, and the evaluation does
    /// not undo a decision — the same rule E5b states for `Withdrawn`. While this
    /// equals the dot the sheet currently reads, no replacement line is opened; a
    /// Resources change or `@wod/economy migrate` clears the mark and the next
    /// evaluation opens the next line.
    #[serde(default)]
    pub income_cancelled_at: Option<u8>,
    /// The last period an opportunity was generated (the `Coast` rate limit).
    #[serde(default)]
    pub last_opportunity_period: u64,
    /// Bounded tail, ≤ [`MAX_PROFILE_HISTORY`].
    #[serde(default)]
    pub history: Vec<ProfileEvent>,
    /// Staleness stamp.
    pub revision: u64,
}

impl SpendingProfile {
    /// A fresh profile for `obj`, at the configured engagement.
    #[must_use]
    pub fn new(obj: ObjId, resources: u8, engagement: Engagement) -> SpendingProfile {
        SpendingProfile {
            obj,
            resources: resources.min(MAX_LEVEL),
            sources: Vec::new(),
            level: resources.min(MAX_LEVEL),
            strain: 0,
            engagement,
            haven: None,
            prompt: None,
            account: None,
            income: None,
            income_cancelled_at: None,
            last_opportunity_period: 0,
            history: Vec::new(),
            revision: 1,
        }
    }

    /// The career source, if there is one.
    #[must_use]
    pub fn career(&self) -> Option<&CareerProfile> {
        self.sources.iter().find_map(|s| match s {
            SupportSource::Career(c) => Some(c.as_ref()),
            _ => None,
        })
    }

    /// The career source, mutably.
    pub fn career_mut(&mut self) -> Option<&mut CareerProfile> {
        self.sources.iter_mut().find_map(|s| match s {
            SupportSource::Career(c) => Some(c.as_mut()),
            _ => None,
        })
    }

    /// The stance in force **this** period (`Coast` with no career).
    #[must_use]
    pub fn stance(&self) -> Stance {
        self.career().map_or(Stance::Coast, |c| c.stance)
    }

    /// The plan sources housing is told about, capped at [`MAX_SOURCES`].
    #[must_use]
    pub fn housing_sources(&self) -> Vec<mush_housing::SupportSource> {
        self.sources.iter().take(MAX_SOURCES).map(SupportSource::to_housing).collect()
    }

    /// Append a bounded history line.
    pub fn note(&mut self, at: i64, what: impl AsRef<str>) {
        self.history.push(ProfileEvent { at, what: SmolStr::new(clip_line(what.as_ref())) });
        if self.history.len() > MAX_PROFILE_HISTORY {
            let drop = self.history.len() - MAX_PROFILE_HISTORY;
            self.history.drain(..drop);
        }
    }

    /// Move strain by `delta`, clamped into `0..=`[`STRAIN_MAX`].
    pub fn apply_strain(&mut self, delta: i16) {
        let next = i16::from(self.strain).saturating_add(delta);
        self.strain = next.clamp(0, i16::from(STRAIN_MAX)) as u8;
    }
}

/// Derive a support level from a Resources rating and a stance modifier, clamped
/// into the `0..=`[`MAX_LEVEL`] band (§5.2).
#[must_use]
pub fn derive_level(resources: u8, delta: i16) -> u8 {
    let n = i16::from(resources.min(MAX_LEVEL)).saturating_add(delta);
    n.clamp(0, i16::from(MAX_LEVEL)) as u8
}

/// The per-faction pool of **currently open** front-attributed prompts, counted
/// across every profile (§5.3).
///
/// It is **derived, never persisted**: boot and every prompt write recompute it from
/// the profiles, so it cannot drift into refusing opportunities for operations that
/// closed. E5 counts its own; the map is the reserved seam for the faction
/// simulation to share.
#[must_use]
pub fn open_pool(profiles: &BTreeMap<ObjId, SpendingProfile>) -> BTreeMap<FactionId, u32> {
    let mut pool: BTreeMap<FactionId, u32> = BTreeMap::new();
    for p in profiles.values() {
        if let Some(f) = p.prompt.as_ref().and_then(|pr| pr.kind.front()) {
            *pool.entry(f.faction).or_default() += 1;
        }
    }
    pool
}

/// Clip a free-text field to [`MAX_TEXT_BYTES`] on a char boundary.
#[must_use]
pub fn clip(s: &str) -> &str {
    let s = s.trim();
    if s.len() <= MAX_TEXT_BYTES {
        return s;
    }
    let mut end = MAX_TEXT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Clip a history line (four fields' worth) on a char boundary.
#[must_use]
fn clip_line(s: &str) -> &str {
    const MAX: usize = MAX_TEXT_BYTES * 4;
    let s = s.trim();
    if s.len() <= MAX {
        return s;
    }
    let mut end = MAX;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
