//! # Changeling: The Dreaming 20th Anniversary (C20) — the Kithain splat
//!
//! A third worked implementation of the [`Splat`] contract (Vampire in
//! [`crate::splats::vampire`] is the reference; this module mirrors its
//! structure, alongside Mage's and Werewolf's, exactly). The Kithain are
//! **sheet-tracked, pool-rolled**: Arts and Realms are name→dots maps, a
//! cantrip rolls the **Art rating alone** through [`roll_expr`](crate::roll::roll_expr)
//! (the real dice engine, roll log, and `wod.roll` panel — the effect itself is
//! ST/player adjudicated). Per C20 the **Realm is a level-gate** on what a
//! cantrip may affect, not a contributor to the dice pool: the caster must have
//! at least one dot in the named Realm, but only the Art's dots are rolled.
//! Glamour is a splat pool with spend/gain/show commands — no bespoke
//! per-cantrip automation.
//!
//! What this module adds on top of the base oWoD sheet:
//!
//! - a **Kith** ([`KITHS`], the 13 canonical kiths plus an open "other"): each
//!   carries a Birthright and a Frailty, derived onto the sheet as text (an
//!   `"other"` kith leaves both blank for the Storyteller to fill in);
//! - a **Seeming** ([`SEEMINGS`]: Childling/Wilder/Grump) driving starting
//!   Glamour and Banality;
//! - a **Court** (Seelie/Unseelie) plus free-text Seelie/Unseelie Legacy
//!   fields (a Kithain's quest/ban pair — Changeling uses Nature/Demeanor,
//!   already on every sheet, and these Legacies in place of the Virtue-driven
//!   morality Vampire uses);
//! - a **House** ([`HOUSES`]), meaningful only when Kith is Sidhe (not
//!   schema-enforced — a non-Sidhe simply leaves it at `"none"`);
//! - **Glamour** (0..10 dots, permanent + a temporary spend pool, mirroring
//!   Werewolf's Rage/Gnosis) and **Banality** (0..10, a single derived rating —
//!   staff-adjustable, not a freebie/XP purchase: it shifts through play, not
//!   point-buy);
//! - **Arts** ([`ARTS`], the eight canonical cantrip verbs, open to others) and
//!   **Realms** ([`REALMS`], the six canonical cantrip nouns, a closed list) as
//!   name→0..5 maps;
//! - commands `+art`/`+arts`, `+realm`/`+realms`, `+cantrip`, `+glamour`,
//!   `+banality`, `+legacy`, all self-gating on the actor being Kithain.
//!
//! ## Chargen model + deliberate deviations (documented per the milestone's
//! "verify against C20 where you can; where unsure, pick the standard reading
//! and document" instruction)
//!
//! - **Arts 3 / Realms 5**: C20 chargen grants 3 dots of Arts and 5 dots of
//!   Realms (the classic Changeling advantage split, recalled from the
//!   corebook rather than re-verified page-by-page). Both are real
//!   [`ChargenContribution`] pools (`MapDots`-sourced, exactly like Vampire's
//!   Disciplines / Mage's Spheres), so overflow beyond the pool costs
//!   freebies.
//! - **Chargen freebie overflow** uses the C20 RAW freebie prices: an Art dot
//!   beyond the 3-dot pool costs **5** freebies, a Realm dot beyond the 5-dot
//!   pool costs **3** (both are the corebook figures). Arts are not
//!   kith-restricted the way Disciplines are clan-restricted, so there is no
//!   in/out-of-kith split.
//! - **XP pricing** (post-approval `+xp/spend`) follows this codebase's
//!   established Discipline/Sphere-style curve rather than a re-verified
//!   corebook page: a new Art = 10, further dots = current × 7; a new Realm =
//!   6, further = current × 4 (lighter — a supporting trait, not a full
//!   "power"); Glamour follows the Gnosis convention, current × 3. A house
//!   rule can retune these multipliers without touching this file's structure.
//! - **Glamour is a fixed grant, not a chargen pool** (mirrors Werewolf's
//!   Rage/Gnosis exactly, including the reasoning in that module's docs):
//!   permanent + temporary Glamour derive from Seeming at
//!   [`finalize_starting`](Changeling::finalize_starting) and are
//!   chargen-locked, so a mortal-identical freebie budget still balances
//!   without a fabricated "dots above N" baseline (Glamour's baseline varies
//!   by Seeming, which [`SplatPoolSource`] cannot express). Raising permanent
//!   Glamour past the Seeming grant is a post-approval XP matter
//!   (`+xp/spend glamour=<n>`).
//! - **Banality is not purchasable** at all (no freebie line, no `xp_max`
//!   claim): it is derived from Seeming and only staff-adjustable
//!   (`+banality/set`), matching the lore (Banality shifts through roleplay
//!   consequences, not a dot-buy).
//! - **Willpower is Seeming-derived** at chargen (overriding the base Courage
//!   derivation): C20 sets starting Willpower directly from the Seeming —
//!   Childling 1, Wilder 2, Grump 5 (the `willpower` field on [`SeemingDef`]) —
//!   not from a Virtue. Like Werewolf's tribal Willpower it is set outright,
//!   `willpower` is chargen-locked, and [`TraitKind::Willpower`] is exempted
//!   from the base freebie line (exactly as Mage does for its flat baseline),
//!   so a mortal-identical freebie budget still balances.
//! - **`+cast` is deliberately NOT registered.** The milestone brief names
//!   `+cast/+cantrip` for cantrip casting, but Mage already owns `+cast`
//!   ([`crate::splats::mage`]) and both splats register their commands into
//!   the same [`Registrar`] at boot — a second `+cast` would collide and fail
//!   `ModError` the instant a game enables Mage and Changeling together (which
//!   `DEFAULT_TEMPLATES` will, once the integration agent wires this splat
//!   in). Only `+cantrip` is registered. **Flagged loudly for the integration
//!   agent**: no shared file was touched to resolve this — it is a naming
//!   choice local to this module.
//! - **Only Arts is wired as the wizard's `wizard_pool_field`** (per the
//!   [`Splat`] contract, a splat exposes exactly one). Realms remains a full
//!   chargen pool reachable through the ordinary generic `<field>:<name>=<n>`
//!   syntax both `+stat` and the chargen wizard's Freebies step already
//!   support for *any* schema map field (see
//!   [`chargen_cmds::apply_stat`](crate::chargen_cmds::apply_stat)) — it just
//!   does not get the wizard Splat-step's bare-name shorthand Arts gets. A
//!   second pool slot would need extending the `Splat` trait itself, which
//!   this agent was told to flag rather than do; noted here for the
//!   integration agent as an optional future trait extension, not something
//!   this splat is blocked on.
//! - **Cantrip difficulty**: the brief describes difficulty as "derived from
//!   target's Banality or a set number," but `+cantrip` (unlike `+roll
//!   contested`) names no target. `+cantrip` therefore defaults to difficulty
//!   6 (the oWoD standard), takes `vs <n>` to name a specific target's
//!   Banality by hand, and `/bunk` (a fully theatrical cast) lowers the
//!   difficulty by 1 (floor 3) — the standard reading, documented rather than
//!   guessed silently.

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, Registrar, Role, match_object, perm,
};
use mush_ruleset::SheetDoc;
use mush_ruleset::schema::{
    FieldDef, FieldType, SchemaError, SectionLayout, SheetSchemaBuilder, SheetSection,
};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::config::{TraitKind, WodConfig};
use crate::schema::Template;
use crate::splats::{ChargenContribution, Splat, SplatPool, SplatPoolSource, is_template};

// ----------------------------------------------------------------------
// Kith crib table
// ----------------------------------------------------------------------

/// One kith's crib entry: id, label, its Birthright (a Kithain gift), and its
/// Frailty (a Kithain curse). `"other"` carries neither — the Storyteller
/// fills them in for a kith outside the canonical 13.
pub struct KithDef {
    /// The `kith` enum id (snake_case).
    pub id: &'static str,
    /// Human label.
    pub label: &'static str,
    /// The kith's Birthright, derived onto the `birthright` sheet field.
    pub birthright: &'static str,
    /// The kith's Frailty, derived onto the `frailty` sheet field.
    pub frailty: &'static str,
}

/// The 13 canonical kiths plus an open `"other"` (crib data table; short
/// paraphrases, not verbatim corebook text).
pub const KITHS: &[KithDef] = &[
    KithDef {
        id: "boggan",
        label: "Boggan",
        birthright: "Tireless, deft-handed workers: a bonus to any task of craft or \
                      housework, and the right tool always seems close to hand.",
        frailty: "Bound by hospitality and their own word: must finish any task begun \
                   and repay any debt or kindness shown them.",
    },
    KithDef {
        id: "clurichaun",
        label: "Clurichaun",
        birthright: "Never suffer for drink: immune to its ill effects and always able \
                      to find good cheer, with a knack for bargains sealed over a glass.",
        frailty: "Compulsive gamblers and drinkers: cannot pass up a wager or refuse a \
                   drink once offered.",
    },
    KithDef {
        id: "eshu",
        label: "Eshu",
        birthright: "Born storytellers and wanderers: never truly lose their way, and a \
                      well-told tale seems to carry a fortune of its own.",
        frailty: "Cannot resist embellishing a good story, even at their own expense, \
                   and grow restless if they linger too long in one place.",
    },
    KithDef {
        id: "ghille_dhu",
        label: "Ghille Dhu",
        birthright: "Kin to green and growing things: can vanish into natural cover and \
                      calm wild plants and animals.",
        frailty: "Crippling shyness among strangers, and a pull toward solitude in wild \
                   places when crowds grow too close.",
    },
    KithDef {
        id: "nocker",
        label: "Nocker",
        birthright: "Master crafters: given time and materials, can build or mend nearly \
                      anything, often brilliantly.",
        frailty: "Foul-tempered perfectionists: a botched working sends them into a \
                   cursing, hammer-throwing rage.",
    },
    KithDef {
        id: "piskey",
        label: "Piskey",
        birthright: "Quick and quiet: a natural gift for stealth, mischief, and slipping \
                      away unseen.",
        frailty: "Kleptomaniac impulses: cannot resist pocketing some small trinket that \
                   catches their eye.",
    },
    KithDef {
        id: "pooka",
        label: "Pooka",
        birthright: "Can take the shape of an animal tied to their nature, moving as \
                      easily on four legs as two.",
        frailty: "Cannot speak a plain, unadorned truth: every honest word comes wrapped \
                   in exaggeration, riddle, or a trickster's spin.",
    },
    KithDef {
        id: "redcap",
        label: "Redcap",
        birthright: "Fearsome and iron-stomached: a bonus resisting fear and \
                      intimidating others, and can stomach almost anything.",
        frailty: "Voracious, violent appetites: a hunger for raw meat and a quick temper \
                   that turns hunger into violence.",
    },
    KithDef {
        id: "satyr",
        label: "Satyr",
        birthright: "Boundless vigor and passion: a bonus to feats of stamina, revelry, \
                      and seduction.",
        frailty: "Slaves to their own appetites: cannot resist strong drink, revelry, or \
                   desire once it is offered.",
    },
    KithDef {
        id: "selkie",
        label: "Selkie",
        birthright: "At home in the water: swims as if born to it, and a sealskin lets \
                      them take seal form.",
        frailty: "Bound to that sealskin: lost or stolen, they are trapped in their \
                   current shape and driven to recover it.",
    },
    KithDef {
        id: "sidhe",
        label: "Sidhe",
        birthright: "Noble bearing and commanding presence: a bonus to social rolls that \
                      trade on rank, beauty, or leadership.",
        frailty: "Bound by a strict Noblesse Oblige: never break their given word or act \
                   in a manner beneath their station.",
    },
    KithDef {
        id: "sluagh",
        label: "Sluagh",
        birthright: "Masters of secrets: a bonus to stealth and eavesdropping, and \
                      shadows always seem to have something worth overhearing.",
        frailty: "A rasping, secret-keeper's voice: raising it above a whisper is an \
                   effort, and few of their words come without a hidden cost.",
    },
    KithDef {
        id: "troll",
        label: "Troll",
        birthright: "Mighty and steadfast: a bonus to feats of strength, and their word \
                      once given is iron.",
        frailty: "Bound utterly to their own oath: breaking a promise costs them dearly, \
                   in Glamour and in spirit.",
    },
    KithDef {
        id: "other",
        label: "Other",
        birthright: "",
        frailty: "",
    },
];

/// The kith definition for an id (case-insensitive).
#[must_use]
pub fn kith_def(id: &str) -> Option<&'static KithDef> {
    KITHS.iter().find(|k| k.id.eq_ignore_ascii_case(id))
}

/// The kith enum options (the ids).
fn kith_ids() -> Vec<SmolStr> {
    KITHS.iter().map(|k| SmolStr::new(k.id)).collect()
}

/// The kith recorded on a sheet, if any.
#[must_use]
pub fn kith_of(doc: &SheetDoc) -> Option<&'static KithDef> {
    doc.text("kith").and_then(kith_def)
}

/// Whether the sheet's kith is Sidhe (the only kith House is meaningful for).
#[must_use]
pub fn is_sidhe(doc: &SheetDoc) -> bool {
    doc.text("kith")
        .is_some_and(|k| k.eq_ignore_ascii_case("sidhe"))
}

// ----------------------------------------------------------------------
// Seeming crib table
// ----------------------------------------------------------------------

/// One seeming's crib entry: id, label, starting Glamour, starting Banality,
/// and starting Willpower — the C20 Seeming chart (Childling / Wilder / Grump).
pub struct SeemingDef {
    /// The `seeming` enum id (snake_case).
    pub id: &'static str,
    /// Human label.
    pub label: &'static str,
    /// Starting Glamour (Childling highest, Grump lowest): 5 / 4 / 3.
    pub glamour: i32,
    /// Starting Banality (Childling lowest, Grump highest): 1 / 3 / 5.
    pub banality: i32,
    /// Starting Willpower (Grump highest, Childling lowest): 1 / 2 / 5.
    pub willpower: i32,
}

/// The three seemings (crib data table — the C20 Seeming chart).
pub const SEEMINGS: &[SeemingDef] = &[
    SeemingDef {
        id: "childling",
        label: "Childling",
        glamour: 5,
        banality: 1,
        willpower: 1,
    },
    SeemingDef {
        id: "wilder",
        label: "Wilder",
        glamour: 4,
        banality: 3,
        willpower: 2,
    },
    SeemingDef {
        id: "grump",
        label: "Grump",
        glamour: 3,
        banality: 5,
        willpower: 5,
    },
];

/// The seeming definition for an id (case-insensitive).
#[must_use]
pub fn seeming_def(id: &str) -> Option<&'static SeemingDef> {
    SEEMINGS.iter().find(|s| s.id.eq_ignore_ascii_case(id))
}

/// The seeming enum options (the ids).
fn seeming_ids() -> Vec<SmolStr> {
    SEEMINGS.iter().map(|s| SmolStr::new(s.id)).collect()
}

/// The seeming recorded on a sheet (falling back to Wilder — the common adult
/// baseline — if unset/unrecognized).
#[must_use]
pub fn seeming_of(doc: &SheetDoc) -> &'static SeemingDef {
    doc.text("seeming")
        .and_then(seeming_def)
        .unwrap_or(&SEEMINGS[1])
}

// ----------------------------------------------------------------------
// Court / House
// ----------------------------------------------------------------------

/// The two Courts.
pub const COURTS: &[&str] = &["seelie", "unseelie"];

/// The Sidhe royal Houses, plus `"none"` for a non-Sidhe (or an unhoused
/// Sidhe) — the default.
pub const HOUSES: &[&str] = &[
    "none", "beaumayn", "dougal", "eiluned", "fiona", "gwydion", "liam", "scathach",
];

fn court_ids() -> Vec<SmolStr> {
    COURTS.iter().map(|c| SmolStr::new(*c)).collect()
}

fn house_ids() -> Vec<SmolStr> {
    HOUSES.iter().map(|h| SmolStr::new(*h)).collect()
}

// ----------------------------------------------------------------------
// Arts / Realms
// ----------------------------------------------------------------------

/// The eight canonical Arts (crib reference list — the map allows others; a
/// Kithain's birthright or a house rule may grant an unlisted Art).
pub const ARTS: &[&str] = &[
    "Chicanery",
    "Legerdemain",
    "Naming",
    "Oneiromancy",
    "Primal",
    "Soothsay",
    "Sovereign",
    "Wayfare",
];

/// The six Realms (a closed list — a cantrip's noun half).
pub const REALMS: &[&str] = &["Actor", "Fae", "Nature", "Prop", "Scene", "Time"];

/// The canonical Realm name for a token (case-insensitive), or `None`.
#[must_use]
pub fn canonical_realm(token: &str) -> Option<&'static str> {
    REALMS
        .iter()
        .copied()
        .find(|r| r.eq_ignore_ascii_case(token))
}

// ----------------------------------------------------------------------
// Freebie / XP cost helpers
// ----------------------------------------------------------------------

/// The XP (and chargen freebie-overflow) cost to raise an Art `from`→`to`: a
/// new Art (0→1) costs a flat 10; further dots cost current × 7 — the
/// Discipline/Sphere "power" convention (see the module docs).
#[must_use]
pub fn art_raise_cost(from: i32, to: i32) -> i32 {
    (from.max(0)..to)
        .map(|r| if r <= 0 { 10 } else { r * 7 })
        .sum()
}

/// The XP (and chargen freebie-overflow) cost to raise a Realm `from`→`to`: a
/// new Realm costs a flat 6; further dots cost current × 4 — a lighter,
/// supporting-trait price (see the module docs).
#[must_use]
pub fn realm_raise_cost(from: i32, to: i32) -> i32 {
    (from.max(0)..to)
        .map(|r| if r <= 0 { 6 } else { r * 4 })
        .sum()
}

/// The XP cost to raise permanent Glamour `from`→`to`: current × 3 (the
/// Gnosis convention; Glamour is not a chargen pool — see the module docs).
#[must_use]
pub fn glamour_raise_cost(from: i32, to: i32) -> i32 {
    // `from.max(0)` with `r.max(1)` inside charges the 0→1 step 1×mult, exactly
    // like the sibling pool helpers (werewolf::pool_raise_cost,
    // kinfolk::gnosis_raise_cost) — a raise from 0 must never be free.
    (from.max(0)..to).map(|r| r.max(1) * 3).sum()
}

// ----------------------------------------------------------------------
// The Changeling splat
// ----------------------------------------------------------------------

/// The C20 splat.
#[derive(Debug, Default, Clone, Copy)]
pub struct Changeling;

/// This splat behind an `Arc<dyn Splat>`, for the resolver arm the integration
/// agent adds (`"changeling" => Some(changeling::template())`).
#[must_use]
pub(crate) fn template() -> Arc<dyn Splat> {
    Arc::new(Changeling)
}

impl Template for Changeling {
    fn id(&self) -> &'static str {
        "changeling"
    }

    fn display_name(&self) -> &str {
        "Changeling"
    }

    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        // Fae identity + pools.
        let fae = SheetSection::new("fae", "Fae", SectionLayout::List)
            .field(FieldDef::enumeration("kith", "Kith", kith_ids()))
            .field(FieldDef::text("kith_other", "Kith (other)", 40))
            .field(
                FieldDef::enumeration("seeming", "Seeming", seeming_ids())
                    .with_default(serde_json::json!("wilder")),
            )
            .field(FieldDef::enumeration("court", "Court", court_ids()))
            .field(FieldDef::enumeration("house", "House", house_ids()))
            .field(FieldDef::text("seelie_legacy", "Seelie Legacy", 40))
            .field(FieldDef::text("unseelie_legacy", "Unseelie Legacy", 40))
            .field(FieldDef::text("birthright", "Birthright", 240))
            .field(FieldDef::text("frailty", "Frailty", 240))
            .field(FieldDef::dots("glamour", "Glamour", 0, 10))
            .field(FieldDef::number(
                "glamour_temp",
                "Glamour (temporary)",
                0,
                10,
            ))
            .field(FieldDef::number("banality", "Banality", 0, 10));
        let b = b.section(fae)?;

        // Arts map (name → 0..5).
        let arts = SheetSection::new("arts", "Arts", SectionLayout::Map).field(FieldDef::map(
            "arts",
            "Arts",
            FieldType::Dots { min: 0, max: 5 },
        ));
        let b = b.section(arts)?;

        // Realms map (name → 0..5).
        let realms = SheetSection::new("realms", "Realms", SectionLayout::Map).field(
            FieldDef::map("realms", "Realms", FieldType::Dots { min: 0, max: 5 }),
        );
        b.section(realms)
    }
}

impl Splat for Changeling {
    fn register_commands(&self, reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
        register_commands(reg, cmd)
    }

    fn chargen_contribution(&self, _config: &WodConfig, _doc: &SheetDoc) -> ChargenContribution {
        ChargenContribution {
            pools: vec![
                SplatPool {
                    id: SmolStr::new("arts"),
                    label: "Arts".to_string(),
                    total: 3,
                    source: SplatPoolSource::MapDots(SmolStr::new("arts")),
                    // C20 RAW: an Art dot beyond the pool costs 5 freebies.
                    freebie_cost: 5,
                },
                SplatPool {
                    id: SmolStr::new("realms"),
                    label: "Realms".to_string(),
                    total: 5,
                    source: SplatPoolSource::MapDots(SmolStr::new("realms")),
                    freebie_cost: 3,
                },
            ],
            // Willpower is Seeming-derived (not the config's Courage baseline),
            // so exempt it from the base Willpower freebie line — mirrors Mage's
            // exemption for its own flat baseline.
            freebie_exempt: vec![TraitKind::Willpower],
        }
    }

    fn finalize_starting(&self, _config: &WodConfig, doc: &mut SheetDoc) {
        // Glamour + Banality + Willpower from Seeming. Glamour's temporary pool
        // starts full.
        let seeming = seeming_of(doc);
        doc.set_number("glamour", i64::from(seeming.glamour));
        doc.set_number("glamour_temp", i64::from(seeming.glamour));
        doc.set_number("banality", i64::from(seeming.banality));

        // Birthright / Frailty text from the kith (blank for "other" — the
        // Storyteller fills them in).
        let (birthright, frailty) = kith_of(doc).map_or(("", ""), |k| (k.birthright, k.frailty));
        doc.set_text("birthright", birthright);
        doc.set_text("frailty", frailty);

        // Willpower: C20 sets the starting rating directly from the Seeming
        // (Childling 1 / Wilder 2 / Grump 5), not the Courage-derived default
        // (see the module docs). Set outright, like Werewolf's tribal Willpower.
        doc.set_number("willpower", i64::from(seeming.willpower));
        doc.set_number("willpower_temp", i64::from(seeming.willpower));
    }

    fn chargen_locked_fields(&self) -> &'static [&'static str] {
        &[
            "glamour",
            "glamour_temp",
            "banality",
            "birthright",
            "frailty",
            "willpower",
        ]
    }

    fn wizard_splat_fields(&self) -> &'static [&'static str] {
        &["kith", "seeming", "court", "house"]
    }

    fn wizard_pool_field(&self) -> Option<&'static str> {
        Some("arts")
    }

    fn wizard_set_pool(
        &self,
        cmd: &WodCmd,
        ctx: &mut Ctx<'_>,
        name: &str,
        rating: i32,
    ) -> Result<(), CmdError> {
        apply_art(cmd, ctx, name, i64::from(rating))
    }

    fn on_field_changed(&self, doc: &mut SheetDoc, field: &str) {
        if field.eq_ignore_ascii_case("kith") {
            let (birthright, frailty) =
                kith_of(doc).map_or(("", ""), |k| (k.birthright, k.frailty));
            doc.set_text("birthright", birthright);
            doc.set_text("frailty", frailty);
        }
    }

    fn xp_max(&self, field: &str, key: Option<&str>) -> Option<i32> {
        match field {
            "arts" | "realms" if key.is_some() => Some(5),
            "glamour" if key.is_none() => Some(10),
            // Banality is deliberately not claimed here: it is never bought
            // with XP (see the module docs).
            _ => None,
        }
    }

    fn xp_cost(
        &self,
        _config: &WodConfig,
        _doc: &SheetDoc,
        field: &str,
        _key: Option<&str>,
        from: i32,
        to: i32,
    ) -> i32 {
        match field {
            "arts" => art_raise_cost(from, to),
            "realms" => realm_raise_cost(from, to),
            "glamour" => glamour_raise_cost(from, to),
            _ => 0,
        }
    }
}

// ----------------------------------------------------------------------
// Commands (all self-gate on the actor being Kithain)
// ----------------------------------------------------------------------

/// Refuse politely if the actor is not a changeling.
fn require_kithain(cmd: &WodCmd, ctx: &Ctx<'_>) -> Result<SheetDoc, CmdError> {
    if !is_template(cmd, ctx, "changeling") {
        return Err(CmdError::msg("You are not Kithain."));
    }
    cmd.read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))
}

/// Set one Art entry to `rating` for the acting changeling — the core of
/// `+art`, shared with the chargen wizard's pool shorthand. Arts are open
/// (any name is allowed; the map has no canonical enforcement — see the
/// module docs), so this is a thin, validated wrapper over the generic
/// `+stat arts:<name>=<n>` path (mirroring Vampire's Discipline setter).
/// Persists; emits no output.
fn apply_art(cmd: &WodCmd, ctx: &mut Ctx<'_>, name: &str, rating: i64) -> Result<(), CmdError> {
    if name.trim().is_empty() {
        return Err(CmdError::msg("Name an Art."));
    }
    if !(0..=5).contains(&rating) {
        return Err(CmdError::msg("An Art is rated 0 to 5."));
    }
    crate::chargen_cmds::apply_stat(cmd, ctx, "arts", Some(name.trim()), &rating.to_string())
}

/// Set one Realm entry to `rating` for the acting changeling — the core of
/// `+realm`. Unlike Arts, Realms are a closed list: the name must match one
/// of the six canonical [`REALMS`]. Persists; emits no output.
fn apply_realm(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    name: &str,
    rating: i64,
) -> Result<&'static str, CmdError> {
    let realm = canonical_realm(name.trim()).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown Realm {name:?}. The six are: {}.",
            REALMS.join(", ")
        ))
    })?;
    if !(0..=5).contains(&rating) {
        return Err(CmdError::msg("A Realm is rated 0 to 5."));
    }
    crate::chargen_cmds::apply_stat(cmd, ctx, "realms", Some(realm), &rating.to_string())?;
    Ok(realm)
}

/// `+art` / `+arts` (list) or `+art <name>=<0..5>` (rate; any name allowed).
fn art_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_string();
    if lhs.is_empty() {
        let doc = require_kithain(cmd, ctx)?;
        return dots_map_list(ctx, &doc, "arts", "Arts");
    }
    let rating: i64 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .ok_or_else(|| CmdError::msg("Usage: +art <name>=<0..5>"))?;
    apply_art(cmd, ctx, &lhs, rating)?;
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("{} set to {rating}.", lhs.trim()));
    Ok(())
}

/// `+realm` / `+realms` (list) or `+realm <name>=<0..5>` (rate; the six
/// canonical Realms only).
fn realm_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_string();
    if lhs.is_empty() {
        let doc = require_kithain(cmd, ctx)?;
        return dots_map_list(ctx, &doc, "realms", "Realms");
    }
    let rating: i64 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .ok_or_else(|| CmdError::msg("Usage: +realm <name>=<0..5>"))?;
    let realm = apply_realm(cmd, ctx, &lhs, rating)?;
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("{realm} set to {rating}."));
    Ok(())
}

/// List a dots-map field's non-zero entries under `label`.
fn dots_map_list(
    ctx: &mut Ctx<'_>,
    doc: &SheetDoc,
    field: &str,
    label: &str,
) -> Result<(), CmdError> {
    ctx.out.sys_to_actor(ctx.actor, "wod", format!("{label}:"));
    let mut any = false;
    if let Some(m) = doc.map(field) {
        let mut entries: Vec<(&String, i64)> = m
            .iter()
            .filter_map(|(k, v)| v.as_i64().map(|n| (k, n)))
            .filter(|(_, n)| *n > 0)
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (k, n) in entries {
            any = true;
            ctx.out
                .sys_to_actor(ctx.actor, "wod", format!("  {k}: {n}"));
        }
    }
    if !any {
        ctx.out.sys_to_actor(ctx.actor, "wod", "  (none)");
    }
    Ok(())
}

/// `+cantrip <art>+<realm> [vs <diff>]` (`/bunk` lowers the difficulty by 1,
/// floor 3) — roll the **Art pool alone**. Per C20 the Realm is a level-gate on
/// what the cantrip may affect (the caster must have at least one dot in it),
/// not a contributor to the dice pool, so both the Art and the Realm are
/// required but only the Art's dots are rolled.
fn cantrip_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_kithain(cmd, ctx)?;
    let (combo, diff) = split_vs(ctx.args.raw());
    let (art_tok, realm_tok) = combo.split_once('+').ok_or_else(|| {
        CmdError::msg(
            "Usage: +cantrip <art>+<realm> [vs <diff>] (e.g. +cantrip chicanery+actor vs 6)",
        )
    })?;
    let art_tok = art_tok.trim();
    let realm_tok = realm_tok.trim();
    if art_tok.is_empty() || realm_tok.is_empty() {
        return Err(CmdError::msg(
            "Usage: +cantrip <art>+<realm> [vs <diff>] (e.g. +cantrip chicanery+actor vs 6)",
        ));
    }
    let realm = canonical_realm(realm_tok).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown Realm {realm_tok:?}. The six are: {}.",
            REALMS.join(", ")
        ))
    })?;
    let art_dots = doc
        .map("arts")
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(art_tok))
                .and_then(|(_, v)| v.as_i64())
        })
        .unwrap_or(0);
    if art_dots < 1 {
        return Err(CmdError::msg(format!(
            "You do not know the Art {art_tok:?}."
        )));
    }
    let realm_dots = doc
        .map("realms")
        .and_then(|m| m.get(realm).and_then(serde_json::Value::as_i64))
        .unwrap_or(0);
    if realm_dots < 1 {
        return Err(CmdError::msg(format!(
            "You have no dots in the Realm of {realm}."
        )));
    }
    let mut difficulty = diff.unwrap_or(6);
    if ctx.switches.has("bunk") {
        difficulty = difficulty.saturating_sub(1).max(3);
    }
    // Roll the Art alone (the Realm gates what may be affected, verified above,
    // but adds no dice — see the C20 note on this fn and the module docs). M14: a
    // cantrip is a power — route it through the director (perceived-or-not per
    // onlooker) instead of broadcasting the raw dice line to the room.
    let actor_name = name_of(ctx, ctx.actor);
    let perc = crate::perception::default_perceptibility(format!(
        "You feel the chill of the Dreaming as {actor_name} weaves a cantrip."
    ));
    crate::roll::roll_expr(
        cmd,
        ctx,
        ctx.actor,
        art_tok,
        difficulty,
        true,
        "cantrip",
        Some(&perc),
        false,
    )?;
    Ok(())
}

/// `+glamour` / `+glamour/spend <n>` / `+glamour/gain <n>`.
fn glamour_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_kithain(cmd, ctx)?;
    let perm = doc.number("glamour").unwrap_or(0) as i32;
    let cur = doc.number("glamour_temp").unwrap_or(0) as i32;

    if ctx.switches.has("spend") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Spend how much Glamour?"));
        }
        if n > cur {
            return Err(CmdError::msg(format!(
                "You have only {cur} Glamour; you cannot spend {n}."
            )));
        }
        set_pool(cmd, ctx, &mut doc, "glamour_temp", cur - n)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!(
                "You spend {n} Glamour weaving a cantrip. Glamour {}/{perm}.",
                cur - n
            ),
        );
        return Ok(());
    }
    if ctx.switches.has("gain") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Gain how much Glamour?"));
        }
        if cur >= perm {
            return Err(CmdError::msg(format!(
                "Your Glamour pool is already full ({perm}). Raise permanent Glamour with \
                 +xp/spend glamour=<n>."
            )));
        }
        let next = (cur + n).min(perm);
        set_pool(cmd, ctx, &mut doc, "glamour_temp", next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!("Wonder and whimsy fill you. Glamour rises to {next}/{perm}."),
        );
        return Ok(());
    }
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Glamour: {cur}/{perm} (temporary/permanent)."),
    );
    Ok(())
}

/// `+banality` (show) / `+banality/set <target>=<n>` (staff adjust).
fn banality_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("set") {
        return banality_set(cmd, ctx);
    }
    let doc = require_kithain(cmd, ctx)?;
    let n = doc.number("banality").unwrap_or(0);
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("Banality: {n}/10."));
    Ok(())
}

/// `+banality/set <target>=<0..10>` — staff adjust a changeling's Banality.
fn banality_set(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !perm::has_role(ctx.w, ctx.actor, Role::Royalty) {
        return Err(CmdError::msg("Permission denied."));
    }
    let target_spec = ctx.args.lhs().trim();
    let n: i64 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .filter(|n| (0..=10).contains(n))
        .ok_or_else(|| CmdError::msg("Usage: +banality/set <target>=<0..10>"))?;
    let target = match_object(ctx.w, ctx.actor, target_spec)?;
    let mut doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    if doc.text("template") != Some("changeling") {
        return Err(CmdError::msg("That character is not Kithain."));
    }
    let store = cmd.store_for_doc(&doc);
    store
        .set_field(&mut doc, "banality", serde_json::json!(n))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Set {}'s Banality to {n}.", name_of(ctx, target)),
    );
    Ok(())
}

/// `+legacy` — show the Seelie and Unseelie Legacy.
fn legacy_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_kithain(cmd, ctx)?;
    let seelie = doc.text("seelie_legacy").unwrap_or("");
    let unseelie = doc.text("unseelie_legacy").unwrap_or("");
    let court = doc.text("court").unwrap_or("seelie");
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("Court: {court}."));
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "Seelie Legacy: {}",
            if seelie.is_empty() { "(unset)" } else { seelie }
        ),
    );
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "Unseelie Legacy: {}",
            if unseelie.is_empty() {
                "(unset)"
            } else {
                unseelie
            }
        ),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

/// Validate + persist a splat pool scalar (Glamour's temporary pool).
fn set_pool(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    doc: &mut SheetDoc,
    field: &str,
    value: i32,
) -> Result<(), CmdError> {
    let store = cmd.store_for_doc(doc);
    store
        .set_field(doc, field, serde_json::json!(value.max(0)))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_in(ctx.actor, doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    Ok(())
}

/// Parse a bare integer argument (falling back to `default` when empty).
fn arg_count(ctx: &Ctx<'_>, default: i32) -> Result<i32, CmdError> {
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Ok(default);
    }
    raw.split_whitespace()
        .next()
        .unwrap_or("")
        .parse::<i32>()
        .map_err(|_| CmdError::msg(format!("Not a number: {raw:?}")))
}

/// Split a raw arg into `(expr, Some(diff))` on a trailing `vs <n>` / `diff <n>`.
fn split_vs(raw: &str) -> (String, Option<u8>) {
    let lower = raw.to_ascii_lowercase();
    for kw in [" vs ", " diff "] {
        if let Some(idx) = lower.rfind(kw) {
            let (head, tail) = raw.split_at(idx);
            let num = tail[kw.len()..].trim();
            if let Ok(n) = num.parse::<u8>() {
                return (head.trim().to_string(), Some(n));
            }
        }
    }
    (raw.trim().to_string(), None)
}

fn name_of(ctx: &Ctx<'_>, who: mush_core::ObjId) -> String {
    ctx.w.get(who).map_or_else(
        || who.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    )
}

fn fmt_errs(errs: &[mush_ruleset::SheetError]) -> String {
    errs.iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Register the Kithain command set.
pub(crate) fn register_commands(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    let art_help = "+arts  --  List your Arts.\n\
        +art <name>=<0..5>  --  Rate an Art (any name is allowed — Arts are open-ended).";
    for name in ["+art", "+arts"] {
        reg.command(
            CommandSpec {
                name: name.into(),
                shape: ArgShape::EqPair,
                help: art_help.into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| art_cmd(&cmd, ctx))
            },
        )?;
    }
    let realm_help = "+realms  --  List your Realms.\n\
        +realm <name>=<0..5>  --  Rate a Realm (the six canonical Realms only: \
        Actor, Fae, Nature, Prop, Scene, Time).";
    for name in ["+realm", "+realms"] {
        reg.command(
            CommandSpec {
                name: name.into(),
                shape: ArgShape::EqPair,
                help: realm_help.into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| realm_cmd(&cmd, ctx))
            },
        )?;
    }
    reg.command(
        CommandSpec {
            name: "+cantrip".into(),
            switches: vec!["bunk".into()],
            shape: ArgShape::Raw,
            help: "+cantrip <art>+<realm> [vs <diff>]  --  Weave a cantrip: roll the Art pool \
                   (the Realm is a level-gate — you need a dot in it, but only the Art's dots \
                   are rolled), e.g. +cantrip chicanery+actor vs 6. Difficulty defaults to 6; \
                   name a target's Banality by hand with vs <n>.\n\
                   +cantrip/bunk <art>+<realm> [vs <diff>]  --  A fully theatrical cast lowers \
                   the difficulty by 1 (floor 3)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| cantrip_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+glamour".into(),
            switches: vec!["spend".into(), "gain".into()],
            shape: ArgShape::Raw,
            help: "+glamour  --  Show your Glamour pool.\n\
                   +glamour/spend <n>  --  Spend Glamour weaving a cantrip.\n\
                   +glamour/gain <n>  --  Recover Glamour (up to your permanent rating)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| glamour_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+banality".into(),
            switches: vec!["set".into()],
            shape: ArgShape::EqPair,
            help: "+banality  --  Show your Banality rating.\n\
                   +banality/set <target>=<0..10>  --  Adjust a character's Banality (staff)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| banality_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+legacy".into(),
            shape: ArgShape::Raw,
            help: "+legacy  --  Show your Court and Seelie/Unseelie Legacy.".into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| legacy_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splats::{Dispatch, MortalSplat};

    /// Build a dispatch over `[mortal, changeling]` without leaning on the
    /// resolver arm (which the integration agent wires up separately).
    fn dispatch() -> Dispatch {
        let splats: Vec<Arc<dyn Splat>> = vec![Arc::new(MortalSplat), Arc::new(Changeling)];
        Dispatch::build("wod", splats)
    }

    fn changeling_doc() -> SheetDoc {
        let d = dispatch();
        let mut doc = SheetDoc::new_default(&d.schemas.schema_for_id("changeling"));
        doc.set_text("template", "changeling");
        doc
    }

    #[test]
    fn schema_has_fae_arts_realms_not_other_splats() {
        let d = dispatch();
        let c = d.schemas.schema_for_id("changeling");
        for id in [
            "kith",
            "seeming",
            "court",
            "house",
            "glamour",
            "glamour_temp",
            "banality",
            "arts",
            "realms",
            "seelie_legacy",
            "unseelie_legacy",
        ] {
            assert!(c.field(id).is_some(), "changeling schema missing {id}");
        }
        // Not a vampire / mage / werewolf.
        assert!(c.field("disciplines").is_none());
        assert!(c.field("bloodpool").is_none());
        assert!(c.field("spheres").is_none());
        assert!(c.field("gifts").is_none());
        // A mortal carries none of the Fae fields.
        let m = d.schemas.schema_for_id("mortal");
        assert!(m.field("kith").is_none());
        assert!(m.field("glamour").is_none());
        assert!(m.field("arts").is_none());
        // Both default docs validate.
        assert!(mush_ruleset::validate(&c, &c.default_data()).is_ok());
        assert!(mush_ruleset::validate(&m, &m.default_data()).is_ok());
    }

    #[test]
    fn kith_table_is_complete() {
        assert_eq!(KITHS.len(), 14, "13 canonical kiths plus other");
        let sidhe = kith_def("sidhe").unwrap();
        assert!(
            sidhe.birthright.to_lowercase().contains("presence") || !sidhe.birthright.is_empty()
        );
        assert!(kith_def("other").unwrap().birthright.is_empty());
        assert!(kith_def("other").unwrap().frailty.is_empty());
        assert!(kith_def("nope").is_none());
    }

    #[test]
    fn seeming_table_matches_crib() {
        assert_eq!(SEEMINGS.len(), 3);
        // C20 Seeming chart: Glamour 5/4/3, Banality 1/3/5, Willpower 1/2/5.
        let childling = seeming_def("childling").unwrap();
        assert_eq!(childling.glamour, 5);
        assert_eq!(childling.banality, 1);
        assert_eq!(childling.willpower, 1);
        let wilder = seeming_def("wilder").unwrap();
        assert_eq!(wilder.glamour, 4);
        assert_eq!(wilder.banality, 3);
        assert_eq!(wilder.willpower, 2);
        let grump = seeming_def("grump").unwrap();
        assert_eq!(grump.glamour, 3);
        assert_eq!(grump.banality, 5);
        assert_eq!(grump.willpower, 5);
        // Unknown/unset falls back to Wilder.
        let doc = changeling_doc();
        assert_eq!(seeming_of(&doc).id, "wilder");
    }

    #[test]
    fn finalize_sets_glamour_and_banality_from_seeming() {
        let mut doc = changeling_doc();
        doc.set_text("seeming", "childling");
        doc.set_text("kith", "pooka");
        Changeling.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(doc.number("glamour"), Some(5));
        assert_eq!(doc.number("glamour_temp"), Some(5));
        assert_eq!(doc.number("banality"), Some(1));
        assert_eq!(doc.number("willpower"), Some(1), "Childling Willpower");
        assert!(
            doc.text("birthright")
                .unwrap()
                .to_lowercase()
                .contains("shape")
        );
        assert!(!doc.text("frailty").unwrap().is_empty());

        let mut doc2 = changeling_doc();
        doc2.set_text("seeming", "grump");
        Changeling.finalize_starting(&WodConfig::default(), &mut doc2);
        assert_eq!(doc2.number("glamour"), Some(3));
        assert_eq!(doc2.number("banality"), Some(5));
        assert_eq!(doc2.number("willpower"), Some(5), "Grump Willpower");
    }

    #[test]
    fn other_kith_leaves_birthright_and_frailty_blank() {
        let mut doc = changeling_doc();
        doc.set_text("kith", "other");
        doc.set_text("kith_other", "Nunnehi");
        Changeling.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(doc.text("birthright"), Some(""));
        assert_eq!(doc.text("frailty"), Some(""));
    }

    #[test]
    fn willpower_is_seeming_derived_at_finalize() {
        // Default seeming (Wilder) → Willpower 2, set outright.
        let mut doc = changeling_doc();
        Changeling.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(doc.number("willpower"), Some(2), "Wilder Willpower");
        assert_eq!(doc.number("willpower_temp"), Some(2));
        // A Grump's Willpower is 5, not a Courage floor: even a pre-existing
        // higher-or-lower value is overwritten by the Seeming rating (like
        // Werewolf's tribal Willpower).
        let mut doc2 = changeling_doc();
        doc2.set_text("seeming", "grump");
        doc2.set_number("willpower", 3); // as if Courage had floored it
        Changeling.finalize_starting(&WodConfig::default(), &mut doc2);
        assert_eq!(doc2.number("willpower"), Some(5), "Grump Willpower is set");
        // A Childling drops to 1, not floored up by Courage.
        let mut doc3 = changeling_doc();
        doc3.set_text("seeming", "childling");
        doc3.set_number("willpower", 4);
        Changeling.finalize_starting(&WodConfig::default(), &mut doc3);
        assert_eq!(
            doc3.number("willpower"),
            Some(1),
            "Childling Willpower is set"
        );
        assert!(Changeling.chargen_locked_fields().contains(&"willpower"));
    }

    #[test]
    fn on_field_changed_refreshes_birthright_on_kith_change() {
        let mut doc = changeling_doc();
        doc.set_text("kith", "troll");
        Changeling.on_field_changed(&mut doc, "kith");
        assert!(
            doc.text("birthright")
                .unwrap()
                .to_lowercase()
                .contains("strength")
        );
        assert!(!doc.text("frailty").unwrap().is_empty());
    }

    #[test]
    fn chargen_contribution_pools_and_freebie_costs() {
        let c = Changeling.chargen_contribution(&WodConfig::default(), &changeling_doc());
        let arts = c.pools.iter().find(|p| p.id == "arts").unwrap();
        assert_eq!(arts.total, 3);
        assert_eq!(arts.freebie_cost, 5);
        let realms = c.pools.iter().find(|p| p.id == "realms").unwrap();
        assert_eq!(realms.total, 5);
        assert_eq!(realms.freebie_cost, 3);

        let mut doc = changeling_doc();
        doc.map_set("arts", "Chicanery", serde_json::json!(2));
        doc.map_set("arts", "Wayfare", serde_json::json!(1));
        doc.map_set("realms", "Actor", serde_json::json!(3));
        assert_eq!(arts.spent(&doc), 3);
        assert_eq!(realms.spent(&doc), 3);
        assert!(c.freebie_exempt.contains(&TraitKind::Willpower));
    }

    #[test]
    fn art_and_realm_raise_costs() {
        // New Art (0->1) is a flat 10; further dots are current x7.
        assert_eq!(art_raise_cost(0, 1), 10);
        assert_eq!(art_raise_cost(2, 3), 14);
        // New Realm (0->1) is a flat 6; further dots are current x4.
        assert_eq!(realm_raise_cost(0, 1), 6);
        assert_eq!(realm_raise_cost(2, 3), 8);
        // Glamour: current x3, and the 0->1 step is charged 1x3 (never free).
        assert_eq!(glamour_raise_cost(0, 1), 3);
        assert_eq!(glamour_raise_cost(0, 2), 6);
        assert_eq!(glamour_raise_cost(3, 4), 9);
    }

    #[test]
    fn xp_claims_arts_realms_and_glamour_only() {
        assert_eq!(Changeling.xp_max("arts", Some("Chicanery")), Some(5));
        assert_eq!(
            Changeling.xp_max("arts", None),
            None,
            "the whole map is not a target"
        );
        assert_eq!(Changeling.xp_max("realms", Some("Actor")), Some(5));
        assert_eq!(Changeling.xp_max("glamour", None), Some(10));
        assert_eq!(
            Changeling.xp_max("banality", None),
            None,
            "Banality is never bought with XP"
        );
    }

    #[test]
    fn locked_fields_cover_derived_traits() {
        let locked = Changeling.chargen_locked_fields();
        for f in [
            "glamour",
            "glamour_temp",
            "banality",
            "birthright",
            "frailty",
            "willpower",
        ] {
            assert!(locked.contains(&f), "{f} must be chargen-locked");
        }
    }

    #[test]
    fn realm_table_is_closed() {
        assert_eq!(REALMS.len(), 6);
        assert_eq!(canonical_realm("actor"), Some("Actor"));
        assert_eq!(canonical_realm("TIME"), Some("Time"));
        assert_eq!(canonical_realm("nope"), None);
        assert_eq!(ARTS.len(), 8);
    }

    // ------------------------------------------------------------------
    // Full-stack tests (a real WodRuleset scoped to [mortal, changeling],
    // driven through mush-testkit + JobsModule with a seeded RNG). The
    // private `WodRuleset::with_splats` constructor is reachable from any
    // module in this crate (Rust module privacy), so this needs no changes
    // to the shared `splats::mod` resolver or `DEFAULT_TEMPLATES`.
    // ------------------------------------------------------------------

    use mush_core::GOD;
    use mush_jobs::JobsModule;
    use mush_ruleset::ApprovalState;
    use mush_testkit::TestWorld;

    use crate::state::WodState;

    const S: u64 = 777;

    fn world(seed: u64) -> (TestWorld, mush_core::ObjId) {
        let splats: Vec<Arc<dyn Splat>> = vec![Arc::new(MortalSplat), Arc::new(Changeling)];
        let ruleset = crate::WodRuleset::with_splats(splats, WodConfig::default(), Some(seed));
        let mut tw = TestWorld::with_modules(vec![Box::new(JobsModule::new()), Box::new(ruleset)]);
        let nick = tw.spawn_player("Nick");
        (tw, nick)
    }

    fn read_sheet(tw: &TestWorld, who: mush_core::ObjId) -> SheetDoc {
        let attr = tw
            .world()
            .attr(who, "sheet:wod")
            .and_then(mush_core::Attr::as_json)
            .expect("a sheet");
        serde_json::from_value(attr.clone()).expect("sheet decodes")
    }

    /// Drive a full, budget-balancing Pooka Wilder chargen.
    fn do_changeling_chargen(tw: &mut TestWorld, who: mush_core::ObjId) {
        tw.run(who, "+chargen");
        tw.run(who, "+chargen/template changeling");
        tw.run(who, "+chargen/attr physical=7 social=5 mental=3");
        tw.run(who, "+chargen/abil talents=13 skills=9 knowledges=5");
        tw.run(who, "+stat kith=pooka");
        tw.run(who, "+stat seeming=wilder");
        tw.run(who, "+stat court=seelie");
        for c in [
            "+stat strength=4",
            "+stat dexterity=4",
            "+stat stamina=2",
            "+stat charisma=3",
            "+stat manipulation=3",
            "+stat appearance=2",
            "+stat perception=2",
            "+stat intelligence=2",
            "+stat wits=2",
        ] {
            tw.run(who, c);
        }
        for c in [
            "+stat alertness=3",
            "+stat athletics=3",
            "+stat expression=3",
            "+stat empathy=2",
            "+stat streetwise=2",
        ] {
            tw.run(who, c);
        }
        for c in ["+stat stealth=3", "+stat performance=3", "+stat larceny=3"] {
            tw.run(who, c);
        }
        for c in ["+stat academics=3", "+stat occult=2"] {
            tw.run(who, c);
        }
        tw.run(who, "+stat backgrounds:Chimera=3");
        tw.run(who, "+stat backgrounds:Contacts=2");
        // Arts: 3 dots.
        tw.run(who, "+stat arts:Chicanery=2");
        tw.run(who, "+stat arts:Wayfare=1");
        // Realms: 5 dots.
        tw.run(who, "+stat realms:Actor=3");
        tw.run(who, "+stat realms:Fae=2");
        // Virtues are not a Changeling chargen pool (only Arts/Realms are),
        // so every dot above the free 1 costs base freebies (2/dot); raise
        // only Courage (3 dots above 1 = 6 freebies), leaving the rest at
        // the default 1 so the budget still balances within 15 freebies.
        tw.run(who, "+stat courage=4");
    }

    #[test]
    fn changeling_full_chargen_derives_pools_and_approves() {
        let (mut tw, nick) = world(S);
        do_changeling_chargen(&mut tw, nick);

        let ev = tw.run(nick, "+chargen/status");
        let seen = tw.seen_by(&ev, nick);
        assert!(
            seen.iter().any(|l| l.contains("Budget balances")),
            "changeling budget must balance: {seen:?}"
        );
        assert!(seen.iter().any(|l| l.contains("Arts") && l.contains("3/3")));
        assert!(
            seen.iter()
                .any(|l| l.contains("Realms") && l.contains("5/5"))
        );

        tw.run(nick, "+chargen/submit");
        let job = match read_sheet(&tw, nick).approved {
            ApprovalState::Pending(j) => j,
            other => panic!("expected Pending, got {other:?}"),
        };
        tw.run(GOD, &format!("+job/approve {job}=Welcome to the Dreaming."));
        let doc = read_sheet(&tw, nick);
        assert!(doc.approved.is_approved());
        assert_eq!(doc.text("template"), Some("changeling"));
        // Derived starting values (Wilder: Glamour 4 / Banality 3 / Willpower 2).
        assert_eq!(doc.number("glamour"), Some(4));
        assert_eq!(doc.number("glamour_temp"), Some(4));
        assert_eq!(doc.number("banality"), Some(3));
        assert_eq!(
            doc.number("willpower"),
            Some(2),
            "Wilder Willpower is 2 (Seeming-derived, not Courage)"
        );
        assert!(!doc.text("birthright").unwrap_or("").is_empty());
        assert!(!doc.text("frailty").unwrap_or("").is_empty());
    }

    #[test]
    fn changeling_commands_refuse_a_mortal() {
        let (mut tw, nick) = world(S);
        for cmd in ["+arts", "+realms", "+glamour", "+banality", "+legacy"] {
            let ev = tw.run(nick, cmd);
            assert!(
                tw.seen_by(&ev, nick)
                    .iter()
                    .any(|l| l.contains("not Kithain")),
                "{cmd} must refuse a mortal"
            );
        }
        let ev = tw.run(nick, "+cantrip chicanery+actor");
        assert!(
            tw.seen_by(&ev, nick)
                .iter()
                .any(|l| l.contains("not Kithain"))
        );
    }

    #[test]
    fn cantrip_rolls_the_art_pool_alone() {
        let (mut tw, nick) = world(S);
        tw.run(GOD, "@sheet/set Nick/template=changeling");
        tw.run(GOD, "@sheet/set Nick/arts:Chicanery=2");
        tw.run(GOD, "@sheet/set Nick/realms:Actor=3");
        let ev = tw.run(nick, "+cantrip chicanery+actor vs 6");
        let outcome = tw
            .world()
            .module_state::<WodState>()
            .expect("WodState")
            .recent_logs(nick, 1)
            .first()
            .expect("a logged roll")
            .outcome
            .clone();
        // C20: the cantrip rolls the Art alone. Chicanery 2 → 2 dice at diff 6;
        // the Realm (Actor 3) gates but adds no dice.
        assert_eq!(outcome.dice.len(), 2);
        assert_eq!(outcome.difficulty, 6);
        assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("cantrip")));

        // An unknown Art is refused.
        let denied = tw.run(nick, "+cantrip sovereign+actor");
        assert!(
            tw.seen_by(&denied, nick)
                .iter()
                .any(|l| l.contains("do not know the Art"))
        );
        // A non-canonical Realm is refused.
        let denied2 = tw.run(nick, "+cantrip chicanery+nonsense");
        assert!(
            tw.seen_by(&denied2, nick)
                .iter()
                .any(|l| l.contains("Unknown Realm"))
        );
        // A canonical Realm the changeling has no dots in is refused (the Realm
        // is a level-gate even though it never joins the pool).
        let no_realm = tw.run(nick, "+cantrip chicanery+time");
        assert!(
            tw.seen_by(&no_realm, nick)
                .iter()
                .any(|l| l.contains("no dots in the Realm of Time"))
        );
        // /bunk lowers the difficulty by 1.
        let ev2 = tw.run(nick, "+cantrip/bunk chicanery+actor vs 6");
        assert!(tw.seen_by(&ev2, nick).iter().any(|l| l.contains("diff 5")));
    }

    #[test]
    fn glamour_spend_and_gain_are_bounded() {
        let (mut tw, nick) = world(S);
        tw.run(GOD, "@sheet/set Nick/template=changeling");
        tw.run(GOD, "@sheet/set Nick/glamour=4");
        tw.run(GOD, "@sheet/set Nick/glamour_temp=1");
        tw.run(nick, "+glamour/gain 10");
        assert_eq!(read_sheet(&tw, nick).number("glamour_temp"), Some(4));
        // Cannot gain past the permanent rating.
        let denied = tw.run(nick, "+glamour/gain 1");
        assert!(
            tw.seen_by(&denied, nick)
                .iter()
                .any(|l| l.contains("already full"))
        );
        tw.run(nick, "+glamour/spend 3");
        assert_eq!(read_sheet(&tw, nick).number("glamour_temp"), Some(1));
        let over = tw.run(nick, "+glamour/spend 5");
        assert!(
            tw.seen_by(&over, nick)
                .iter()
                .any(|l| l.contains("you cannot spend"))
        );
    }

    #[test]
    fn banality_shows_and_staff_can_adjust() {
        let (mut tw, nick) = world(S);
        tw.run(GOD, "@sheet/set Nick/template=changeling");
        tw.run(GOD, "@sheet/set Nick/banality=5");
        let ev = tw.run(nick, "+banality");
        assert!(tw.seen_by(&ev, nick).iter().any(|l| l.contains("5/10")));
        tw.run(GOD, "+banality/set Nick=8");
        assert_eq!(read_sheet(&tw, nick).number("banality"), Some(8));
    }
}
