//! # Werewolf: The Apocalypse 20th Anniversary (W20) — the Garou splat
//!
//! The second worked implementation of the [`Splat`] contract (Vampire in
//! [`crate::splats::vampire`] is the reference; this module mirrors its
//! structure exactly). Garou are **sheet-tracked, pool-rolled**: Gifts are a
//! name→level map rolled as the pool the Gift names, the splat pools
//! (Rage / Gnosis) get spend/gain/show commands, Renown is tracked with a
//! Rank helper, and Forms apply their Physical deltas at roll time — no
//! bespoke per-Gift automation.
//!
//! What this module adds on top of the base oWoD sheet:
//!
//! - a **Breed** ([`BREEDS`]: Homid/Metis/Lupus) setting starting Gnosis and a
//!   breed Gift, Metis carrying a birth flaw;
//! - an **Auspice** ([`AUSPICES`], the five moons) setting starting Rage, the
//!   initial Renown allotment, and an auspice Gift;
//! - a **Tribe** ([`TRIBES`], 13) setting starting Willpower, a tribal Gift and
//!   the tribe's ban;
//! - **Rage** and **Gnosis** pools with spend/gain/show commands;
//! - **Renown** (Glory/Honor/Wisdom, temporary + permanent) driving **Rank**;
//! - a **Gifts** map (name→1..5) with a pool-building `+gift/roll`;
//! - the five **Forms** with `+shift`, whose Physical deltas the base `+roll`
//!   reflects (see [`current_form_pool_delta`]);
//! - commands `+rage`, `+gnosis`, `+renown`, `+gifts`/`+gift`, `+shift`,
//!   `+rank`, all self-gating on the actor being Garou.
//!
//! ## Chargen model (a deliberate deviation from the crib's freebie list)
//!
//! Unlike Vampire — whose Disciplines (3) and Virtues (7) are real priority
//! point-buy pools — W20 chargen has **no** power pool: Rage, Gnosis, Willpower,
//! Renown and the three starting Gifts are *fixed grants* from Breed / Auspice /
//! Tribe. This splat therefore adds **no** [`ChargenContribution`] pools and
//! derives every splat value at [`finalize_starting`](Werewolf::finalize_starting),
//! exactly as Vampire derives its Blood Pool. Those fields are chargen-locked
//! (see [`chargen_locked_fields`](Werewolf::chargen_locked_fields)), so a mortal-
//! identical freebie budget still balances. The crib's freebie prices for raising
//! Rage/Gnosis/Gifts *at chargen* are consequently not wired in — raising them is
//! a post-chargen XP / staff matter (`+xp/spend rage=…`, `@sheet/set`). Doing it
//! at chargen would need either a new `SplatPoolSource` (a value that is derived
//! at finalize cannot balance an underspend check before finalize runs) or a
//! per-splat Willpower baseline in `chargen::freebie_spent`; both touch shared
//! code this phase leaves alone.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, Registrar};
use mush_ruleset::SheetDoc;
use mush_ruleset::schema::{
    FieldDef, FieldType, SchemaError, SectionLayout, SheetSchemaBuilder, SheetSection,
};
use smol_str::SmolStr;

use crate::WodCmd;
use crate::config::WodConfig;
use crate::schema::Template;
use crate::splats::{Splat, is_template};

// ----------------------------------------------------------------------
// Breed / Auspice / Tribe crib data tables
// ----------------------------------------------------------------------

/// One breed's crib entry: id, label, starting Gnosis, its canonical level-1
/// Gifts, and (Metis) a birth flaw.
pub struct BreedDef {
    /// The `breed` enum id (snake_case).
    pub id: &'static str,
    /// Human label.
    pub label: &'static str,
    /// Starting Gnosis (Homid 1, Metis 3, Lupus 5).
    pub gnosis: i32,
    /// Canonical level-1 breed Gifts (the first is the free starting one).
    pub gifts: &'static [&'static str],
    /// A birth flaw text (Metis only; empty otherwise).
    pub flaw: &'static str,
}

/// The three breeds (crib data table).
pub const BREEDS: &[BreedDef] = &[
    BreedDef {
        id: "homid",
        label: "Homid",
        gnosis: 1,
        gifts: &["Persuasion", "Smell of Man", "Master of Fire"],
        flaw: "",
    },
    BreedDef {
        id: "metis",
        label: "Metis",
        gnosis: 3,
        gifts: &["Create Element", "Primal Anger", "Sense Wyrm"],
        flaw: "Sterile and born deformed: a Metis carries one permanent physical or \
               mental deformity (Storyteller-assigned).",
    },
    BreedDef {
        id: "lupus",
        label: "Lupus",
        gnosis: 5,
        gifts: &["Hare's Leap", "Heightened Senses", "Sense Prey"],
        flaw: "",
    },
];

/// One auspice's crib entry: id, label, starting Rage, starting Renown
/// allotment `(glory, honor, wisdom)`, and its canonical level-1 Gifts.
pub struct AuspiceDef {
    /// The `auspice` enum id (snake_case).
    pub id: &'static str,
    /// Human label.
    pub label: &'static str,
    /// Starting Rage (Ragabash 1 … Ahroun 5).
    pub rage: i32,
    /// Starting permanent Renown: `(glory, honor, wisdom)`.
    pub renown: (i32, i32, i32),
    /// Canonical level-1 auspice Gifts (the first is the free starting one).
    pub gifts: &'static [&'static str],
}

/// The five auspices (moon phases), in seniority-of-birth order (crib table).
pub const AUSPICES: &[AuspiceDef] = &[
    AuspiceDef {
        id: "ragabash",
        label: "Ragabash",
        rage: 1,
        renown: (1, 1, 1),
        gifts: &[
            "Blur of the Milky Eye",
            "Open Seal",
            "Scent of Running Water",
        ],
    },
    AuspiceDef {
        id: "theurge",
        label: "Theurge",
        rage: 2,
        renown: (0, 0, 3),
        gifts: &["Mother's Touch", "Sense Wyrm", "Spirit Speech"],
    },
    AuspiceDef {
        id: "philodox",
        label: "Philodox",
        rage: 3,
        renown: (0, 3, 0),
        gifts: &["Resist Pain", "Scent of the True Form", "Truth of Gaia"],
    },
    AuspiceDef {
        id: "galliard",
        label: "Galliard",
        rage: 4,
        renown: (2, 0, 1),
        gifts: &["Beast Speech", "Call of the Wyld", "Mindspeak"],
    },
    AuspiceDef {
        id: "ahroun",
        label: "Ahroun",
        rage: 5,
        renown: (3, 0, 0),
        gifts: &["Falling Touch", "Inspiration", "Razor Claws"],
    },
];

/// One tribe's crib entry: id, label, starting Willpower, canonical level-1
/// Gifts, and the tribal ban.
pub struct TribeDef {
    /// The `tribe` enum id (snake_case).
    pub id: &'static str,
    /// Human label.
    pub label: &'static str,
    /// Starting Willpower (the tribal data table).
    pub willpower: i32,
    /// Canonical level-1 tribal Gifts (the first is the free starting one).
    pub gifts: &'static [&'static str],
    /// The tribe's ban (litany / totem obligation text).
    pub ban: &'static str,
}

/// The 13 tribes (crib data table). Willpower follows the W20 tribal table
/// (the ceiling is 4 — Bone Gnawers, Children of Gaia, Stargazers, Wendigo;
/// most tribes, Silver Fangs among them, start at 3).
pub const TRIBES: &[TribeDef] = &[
    TribeDef {
        id: "black_furies",
        label: "Black Furies",
        willpower: 3,
        gifts: &["Breath of the Wyld", "Heightened Senses", "Sense Wyrm"],
        ban: "Defend the wild places and Gaia's daughters; never suffer a rape or its \
              maker to go unanswered.",
    },
    TribeDef {
        id: "bone_gnawers",
        label: "Bone Gnawers",
        willpower: 4,
        gifts: &["Cooking", "Resist Toxin", "Scent of Sweet Honey"],
        ban: "Share what you have with the pack and the streets; never scorn the lowly.",
    },
    TribeDef {
        id: "children_of_gaia",
        label: "Children of Gaia",
        willpower: 4,
        gifts: &["Mercy", "Mother's Touch", "Resist Pain"],
        ban: "Seek peace among the Garou and heal the wounds of the Nation.",
    },
    TribeDef {
        id: "fianna",
        label: "Fianna",
        willpower: 3,
        gifts: &["Faerie Light", "Persuasion", "Resist Toxin"],
        ban: "Honour the old songs and the fae; never break a sworn oath.",
    },
    TribeDef {
        id: "get_of_fenris",
        label: "Get of Fenris",
        willpower: 3,
        gifts: &["Razor Claws", "Resist Pain", "Visage of Fenris"],
        ban: "Show courage in all things; never refuse a fair challenge from the Wyrm's foes.",
    },
    TribeDef {
        id: "glass_walkers",
        label: "Glass Walkers",
        willpower: 3,
        gifts: &["Control Simple Machine", "Diagnostics", "Trick Shot"],
        ban: "Master the ways of the city and the machine; do not despise the works of man.",
    },
    TribeDef {
        id: "red_talons",
        label: "Red Talons",
        willpower: 3,
        gifts: &["Beast Speech", "Scent of Running Water", "Wolf at the Door"],
        ban: "Suffer no humans to spread; live as a wolf, apart from the Weaver's cages.",
    },
    TribeDef {
        id: "shadow_lords",
        label: "Shadow Lords",
        willpower: 3,
        gifts: &["Aura of Confidence", "Fatal Flaw", "Seizing the Edge"],
        ban: "Seize every advantage and test the strong; never let a slight go unrepaid.",
    },
    TribeDef {
        id: "silent_striders",
        label: "Silent Striders",
        willpower: 3,
        gifts: &["Sense Wyrm", "Silence", "Speech of the World"],
        ban: "Carry word between the septs and never rest too long in one place.",
    },
    TribeDef {
        id: "silver_fangs",
        label: "Silver Fangs",
        willpower: 3,
        gifts: &["Falcon's Grasp", "Lambent Flame", "Sense Wyrm"],
        ban: "Lead the Nation with honour worthy of your blood; never abandon your duty.",
    },
    TribeDef {
        id: "stargazers",
        label: "Stargazers",
        willpower: 4,
        gifts: &["Balance", "Channeling", "Sense Wyrm"],
        ban: "Seek wisdom and inner stillness; question, and know thyself.",
    },
    TribeDef {
        id: "uktena",
        label: "Uktena",
        willpower: 3,
        gifts: &["Sense Magic", "Shroud", "Spirit Speech"],
        ban: "Guard the old secrets and the bound spirits; share them with none unworthy.",
    },
    TribeDef {
        id: "wendigo",
        label: "Wendigo",
        willpower: 4,
        gifts: &["Call the Breeze", "Camouflage", "Resist Pain"],
        ban: "Defend the pure land and its people; give the invader no quarter.",
    },
];

/// The breed definition for an id (case-insensitive).
#[must_use]
pub fn breed_def(id: &str) -> Option<&'static BreedDef> {
    BREEDS.iter().find(|b| b.id.eq_ignore_ascii_case(id))
}

/// The auspice definition for an id (case-insensitive).
#[must_use]
pub fn auspice_def(id: &str) -> Option<&'static AuspiceDef> {
    AUSPICES.iter().find(|a| a.id.eq_ignore_ascii_case(id))
}

/// The tribe definition for an id (case-insensitive).
#[must_use]
pub fn tribe_def(id: &str) -> Option<&'static TribeDef> {
    TRIBES.iter().find(|t| t.id.eq_ignore_ascii_case(id))
}

fn breed_ids() -> Vec<SmolStr> {
    BREEDS.iter().map(|b| SmolStr::new(b.id)).collect()
}

fn auspice_ids() -> Vec<SmolStr> {
    AUSPICES.iter().map(|a| SmolStr::new(a.id)).collect()
}

fn tribe_ids() -> Vec<SmolStr> {
    TRIBES.iter().map(|t| SmolStr::new(t.id)).collect()
}

// ----------------------------------------------------------------------
// Renown → Rank
// ----------------------------------------------------------------------

/// The total permanent Renown → Rank thresholds (crib data table). A starting
/// Garou (one auspice allotment, 3 total) is a Rank-1 Cliath.
#[must_use]
pub fn rank_for(total_permanent_renown: i32) -> (i32, &'static str) {
    match total_permanent_renown {
        n if n >= 26 => (5, "Elder"),
        n if n >= 19 => (4, "Athro"),
        n if n >= 13 => (3, "Adren"),
        n if n >= 7 => (2, "Fostern"),
        n if n >= 3 => (1, "Cliath"),
        _ => (0, "Cub"),
    }
}

/// The total permanent Renown recorded on a sheet.
#[must_use]
fn permanent_renown(doc: &SheetDoc) -> i32 {
    ["glory_perm", "honor_perm", "wisdom_perm"]
        .iter()
        .map(|f| doc.number(f).unwrap_or(0) as i32)
        .sum()
}

// ----------------------------------------------------------------------
// Forms
// ----------------------------------------------------------------------

/// The five Garou forms, in the classic order Homid → Lupus.
pub const FORMS: &[&str] = &["homid", "glabro", "crinos", "hispo", "lupus"];

/// A form's stat modifiers (W20 form table). `app` is the Appearance rule:
/// `Some(delta)` is a relative shift; `None` zeroes Appearance (the monstrous
/// war-forms). The Delirium (the Curse) is provoked by the **Crinos** form
/// alone — see [`shift_cmd`]; Hispo is a dire wolf and does not break the Veil.
struct FormMods {
    str_: i32,
    dex: i32,
    sta: i32,
    man: i32,
    app: Option<i32>,
}

/// The W20 form modifier table.
fn form_mods(form: &str) -> FormMods {
    match form.to_ascii_lowercase().as_str() {
        "glabro" => FormMods {
            str_: 2,
            dex: 0,
            sta: 2,
            man: -1,
            app: Some(-1),
        },
        "crinos" => FormMods {
            str_: 4,
            dex: 1,
            sta: 3,
            man: -3,
            app: None,
        },
        "hispo" => FormMods {
            str_: 3,
            dex: 2,
            sta: 3,
            man: -3,
            app: None,
        },
        "lupus" => FormMods {
            str_: 1,
            dex: 2,
            sta: 2,
            man: 0,
            app: Some(0),
        },
        // Homid (or anything unknown) is the base form.
        _ => FormMods {
            str_: 0,
            dex: 0,
            sta: 0,
            man: 0,
            app: Some(0),
        },
    }
}

/// Whether `form` is a valid Garou form id.
#[must_use]
pub fn is_form(form: &str) -> bool {
    FORMS.iter().any(|f| f.eq_ignore_ascii_case(form))
}

/// The pool adjustment the actor's **current form** applies to a resolved
/// `+roll`, given the trait fields the pool touched. This is how a Garou's
/// war-form Physical bonuses (Crinos Strength +4, …) reach the base `+roll`
/// command without the roll layer knowing about Werewolf: [`crate::roll`] calls
/// this after resolving a pool and adds the result to the dice count. Returns 0
/// for any sheet that is not a Garou (so mortals / vampires are untouched).
///
/// Deltas are applied **at roll time**, never stored, because a war form can
/// push a Physical Attribute past the sheet's 5-dot cap (which the store would
/// reject) and because the base rating must stay the character's true value for
/// XP and chargen. Only the Attributes actually named in the pool are shifted:
/// a Manipulation pool is penalised, a Strength pool boosted, an Appearance pool
/// zeroed in the war forms; abilities and everything else pass through.
#[must_use]
pub fn current_form_pool_delta(doc: &SheetDoc, trait_fields: &[String]) -> i32 {
    if doc.text("template") != Some("werewolf") {
        return 0;
    }
    let mods = form_mods(doc.text("current_form").unwrap_or("homid"));
    let touches = |name: &str| trait_fields.iter().any(|f| f.eq_ignore_ascii_case(name));
    let mut delta = 0;
    if touches("strength") {
        delta += mods.str_;
    }
    if touches("dexterity") {
        delta += mods.dex;
    }
    if touches("stamina") {
        delta += mods.sta;
    }
    if touches("manipulation") {
        delta += mods.man;
    }
    if touches("appearance") {
        delta += match mods.app {
            Some(d) => d,
            // War forms: Appearance drops to 0, i.e. remove every dot.
            None => -(doc.number("appearance").unwrap_or(0) as i32),
        };
    }
    delta
}

/// A human summary of a form's stat deltas, for `+shift` / help.
fn form_delta_summary(form: &str) -> String {
    let m = form_mods(form);
    let mut parts: Vec<String> = Vec::new();
    let push = |parts: &mut Vec<String>, label: &str, d: i32| {
        if d != 0 {
            parts.push(format!("{label} {d:+}"));
        }
    };
    push(&mut parts, "Str", m.str_);
    push(&mut parts, "Dex", m.dex);
    push(&mut parts, "Sta", m.sta);
    push(&mut parts, "Man", m.man);
    match m.app {
        Some(d) => push(&mut parts, "App", d),
        None => parts.push("App 0 (monstrous)".to_string()),
    }
    if parts.is_empty() {
        "no stat changes".to_string()
    } else {
        parts.join(", ")
    }
}

// ----------------------------------------------------------------------
// XP costs
// ----------------------------------------------------------------------

/// The XP cost to raise a Gift `from`→`to` (each new level L costs L×3, the W20
/// affinity price; non-affinity distinctions are ST-adjudicated).
#[must_use]
pub fn gift_raise_cost(from: i32, to: i32) -> i32 {
    (from.max(0)..to).map(|r| (r + 1) * 3).sum()
}

/// The XP cost to raise Rage or Gnosis `from`→`to` at `mult` per step (current
/// rating × mult; a first dot from 0 is treated as ×1 of the multiplier).
#[must_use]
fn pool_raise_cost(from: i32, to: i32, mult: i32) -> i32 {
    (from.max(0)..to).map(|r| r.max(1) * mult).sum()
}

// ----------------------------------------------------------------------
// The Werewolf splat
// ----------------------------------------------------------------------

/// The W20 (Garou) splat.
#[derive(Debug, Default, Clone, Copy)]
pub struct Werewolf;

impl Template for Werewolf {
    fn id(&self) -> &'static str {
        "werewolf"
    }

    fn display_name(&self) -> &str {
        "Werewolf"
    }

    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        // Garou identity + pools.
        let garou = SheetSection::new("garou", "Garou", SectionLayout::List)
            .field(FieldDef::enumeration("breed", "Breed", breed_ids()))
            .field(FieldDef::enumeration("auspice", "Auspice", auspice_ids()))
            .field(FieldDef::enumeration("tribe", "Tribe", tribe_ids()))
            .field(FieldDef::number("rank", "Rank", 0, 5))
            .field(FieldDef::number("rage", "Rage", 0, 10))
            .field(FieldDef::number("rage_temp", "Rage (temporary)", 0, 10))
            .field(FieldDef::number("gnosis", "Gnosis", 0, 10))
            .field(FieldDef::number("gnosis_temp", "Gnosis (temporary)", 0, 10))
            .field(
                FieldDef::enumeration(
                    "current_form",
                    "Current Form",
                    FORMS.iter().map(|f| SmolStr::new(*f)).collect(),
                )
                .with_default(serde_json::json!("homid")),
            )
            .field(FieldDef::text("ban", "Tribal Ban", 240))
            .field(FieldDef::text("deformity", "Birth Flaw", 160));
        let b = b.section(garou)?;

        // Renown: temporary pool + permanent rating per category.
        let renown = SheetSection::new("renown", "Renown", SectionLayout::List)
            .field(FieldDef::number("glory_perm", "Glory (permanent)", 0, 10))
            .field(FieldDef::number("glory_temp", "Glory (temporary)", 0, 10))
            .field(FieldDef::number("honor_perm", "Honor (permanent)", 0, 10))
            .field(FieldDef::number("honor_temp", "Honor (temporary)", 0, 10))
            .field(FieldDef::number("wisdom_perm", "Wisdom (permanent)", 0, 10))
            .field(FieldDef::number("wisdom_temp", "Wisdom (temporary)", 0, 10));
        let b = b.section(renown)?;

        // Gifts map (name → level 1..5).
        let gifts = SheetSection::new("gifts", "Gifts", SectionLayout::Map).field(FieldDef::map(
            "gifts",
            "Gifts",
            FieldType::Dots { min: 0, max: 5 },
        ));
        b.section(gifts)
    }
}

impl Splat for Werewolf {
    fn register_commands(&self, reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
        register_commands(reg, cmd)
    }

    // No chargen priority pools: W20 grants Rage/Gnosis/Willpower/Renown/Gifts
    // from Breed/Auspice/Tribe at finalize (see the module docs). The default
    // `chargen_contribution` (an empty contribution) is exactly right, so a
    // Garou budget balances byte-identically to a mortal's.

    fn finalize_starting(&self, _config: &WodConfig, doc: &mut SheetDoc) {
        // Rage from Auspice, Gnosis from Breed. Each is a *permanent* rating
        // plus a *temporary* pool (like Willpower); a fresh Garou starts with
        // the pool full. Spend/gain operate on the temporary field and can
        // never inflate the permanent one — that is an XP matter.
        let rage = auspice_of(doc).map_or(1, |a| a.rage);
        let gnosis = breed_of(doc).map_or(1, |b| b.gnosis);
        doc.set_number("rage", i64::from(rage));
        doc.set_number("rage_temp", i64::from(rage));
        doc.set_number("gnosis", i64::from(gnosis));
        doc.set_number("gnosis_temp", i64::from(gnosis));

        // Willpower is the Tribe's fixed value (W20 sets it *directly*, not as
        // a floor over Courage). Willpower is chargen-locked for Garou, so a
        // player cannot inflate it by pumping Courage; post-chargen it rises
        // through +xp. `willpower_temp` starts full.
        if let Some(tribe) = tribe_of(doc) {
            let wp = i64::from(tribe.willpower);
            doc.set_number("willpower", wp);
            doc.set_number("willpower_temp", wp);
        }

        // Starting permanent Renown from the Auspice; Rank derives from it.
        let (glory, honor, wisdom) = auspice_of(doc).map_or((0, 0, 0), |a| a.renown);
        doc.set_number("glory_perm", i64::from(glory));
        doc.set_number("honor_perm", i64::from(honor));
        doc.set_number("wisdom_perm", i64::from(wisdom));
        let (rank, _) = rank_for(glory + honor + wisdom);
        doc.set_number("rank", i64::from(rank));

        // Ban / birth flaw text, and a homid starting form.
        doc.set_text("ban", tribe_of(doc).map_or("", |t| t.ban));
        doc.set_text("deformity", breed_of(doc).map_or("", |b| b.flaw));
        doc.set_text("current_form", "homid");

        // One free level-1 Gift from each of Breed / Auspice / Tribe.
        for name in [
            breed_of(doc).and_then(|b| b.gifts.first().copied()),
            auspice_of(doc).and_then(|a| a.gifts.first().copied()),
            tribe_of(doc).and_then(|t| t.gifts.first().copied()),
        ]
        .into_iter()
        .flatten()
        {
            doc.map_set("gifts", name, serde_json::json!(1));
        }
    }

    fn chargen_locked_fields(&self) -> &'static [&'static str] {
        // Everything the splat derives at finalize (Breed/Auspice/Tribe stay
        // player-settable); `current_form` is driven by `+shift`, not `+stat`.
        // `willpower` is the fixed tribal value (not Courage-derived), so it is
        // locked here too — post-chargen it rises through +xp.
        &[
            "rage",
            "rage_temp",
            "gnosis",
            "gnosis_temp",
            "willpower",
            "rank",
            "current_form",
            "ban",
            "deformity",
            "gifts",
            "glory_perm",
            "glory_temp",
            "honor_perm",
            "honor_temp",
            "wisdom_perm",
            "wisdom_temp",
        ]
    }

    fn wizard_splat_fields(&self) -> &'static [&'static str] {
        // Breed / Auspice / Tribe drive every derived Garou trait at finalize
        // (Rage, Gnosis, Willpower, Renown, free Gifts); there is no
        // player-allocated splat pool.
        &["breed", "auspice", "tribe"]
    }

    fn on_field_changed(&self, doc: &mut SheetDoc, field: &str) {
        match field.to_ascii_lowercase().as_str() {
            "tribe" => doc.set_text("ban", tribe_of(doc).map_or("", |t| t.ban)),
            "breed" => doc.set_text("deformity", breed_of(doc).map_or("", |b| b.flaw)),
            // Staff edits to permanent Renown re-derive Rank.
            "glory_perm" | "honor_perm" | "wisdom_perm" => {
                let (rank, _) = rank_for(permanent_renown(doc));
                doc.set_number("rank", i64::from(rank));
            }
            _ => {}
        }
    }

    fn xp_max(&self, field: &str, key: Option<&str>) -> Option<i32> {
        match field {
            "gifts" if key.is_some() => Some(5),
            "rage" | "gnosis" if key.is_none() => Some(10),
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
            "gifts" => gift_raise_cost(from, to),
            "gnosis" => pool_raise_cost(from, to, 2),
            // Rage (and any other splat scalar we claimed) at ×1.
            _ => pool_raise_cost(from, to, 1),
        }
    }
}

/// The breed recorded on a sheet, if any.
#[must_use]
pub fn breed_of(doc: &SheetDoc) -> Option<&'static BreedDef> {
    doc.text("breed").and_then(breed_def)
}

/// The auspice recorded on a sheet, if any.
#[must_use]
pub fn auspice_of(doc: &SheetDoc) -> Option<&'static AuspiceDef> {
    doc.text("auspice").and_then(auspice_def)
}

/// The tribe recorded on a sheet, if any.
#[must_use]
pub fn tribe_of(doc: &SheetDoc) -> Option<&'static TribeDef> {
    doc.text("tribe").and_then(tribe_def)
}

// ----------------------------------------------------------------------
// Commands (all self-gate on the actor being Garou)
// ----------------------------------------------------------------------

/// Refuse politely if the actor is not a werewolf.
fn require_garou(cmd: &WodCmd, ctx: &Ctx<'_>) -> Result<SheetDoc, CmdError> {
    if !is_template(cmd, ctx, "werewolf") {
        return Err(CmdError::msg("You are not Garou."));
    }
    cmd.read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))
}

/// Refuse politely unless the actor is Garou **or Kinfolk** — the two W20
/// templates that share Gnosis and Gifts. Used by `+gnosis` and `+gift`
/// (which Kinfolk reuse rather than re-register — see
/// [`crate::splats::kinfolk`]); the strictly-Garou commands (`+rage`,
/// `+renown`, `+shift`, `+rank`) keep [`require_garou`]. Both handlers are
/// already schema-generic — they read `gnosis`/`gnosis_temp`/`gifts` off
/// whatever sheet is passed — so no further Kinfolk-specific branching is
/// needed here.
fn require_garou_or_kin(cmd: &WodCmd, ctx: &Ctx<'_>) -> Result<SheetDoc, CmdError> {
    if !is_template(cmd, ctx, "werewolf") && !is_template(cmd, ctx, "kinfolk") {
        return Err(CmdError::msg("You are not Garou or Kinfolk."));
    }
    cmd.read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))
}

/// `+rage` / `+rage/spend <n>` / `+rage/gain <n>` / `+rage/frenzy [vs <diff>]`.
fn rage_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_garou(cmd, ctx)?;
    if ctx.switches.has("frenzy") {
        return rage_frenzy(cmd, ctx, &doc);
    }
    let perm = doc.number("rage").unwrap_or(0) as i32;
    let cur = doc.number("rage_temp").unwrap_or(0) as i32;
    if ctx.switches.has("spend") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Spend how much Rage?"));
        }
        if n > cur {
            return Err(CmdError::msg(format!(
                "You have only {cur} Rage; you cannot spend {n}."
            )));
        }
        set_pool(cmd, ctx, &mut doc, "rage_temp", cur - n)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!(
                "You unleash {n} Rage for {n} extra action(s). Rage {}/{perm}. (Each extra \
                 action past the first suffers a cumulative dice penalty; the Storyteller \
                 sets the cap.)",
                cur - n
            ),
        );
        return Ok(());
    }
    if ctx.switches.has("gain") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Gain how much Rage?"));
        }
        // Temporary Rage recovers only up to the permanent rating; raising the
        // permanent rating is an XP matter (+xp/spend rage=…), never free.
        if cur >= perm {
            return Err(CmdError::msg(format!(
                "Your Rage pool is already full ({perm}). Raise permanent Rage with \
                 +xp/spend rage=<n>."
            )));
        }
        let next = (cur + n).min(perm);
        set_pool(cmd, ctx, &mut doc, "rage_temp", next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!("Your Rage rises to {next}/{perm}."),
        );
        return Ok(());
    }
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Rage: {cur}/{perm} (temporary/permanent)."),
    );
    Ok(())
}

/// `+rage/frenzy [vs <diff>]` — roll Willpower to resist frenzy. (Werewolf
/// frenzy lives under `+rage` because Vampire already owns the `+frenzy`
/// command name.)
fn rage_frenzy(cmd: &WodCmd, ctx: &mut Ctx<'_>, doc: &SheetDoc) -> Result<(), CmdError> {
    let (_, diff) = split_vs(ctx.args.raw());
    let wp = doc.number("willpower").unwrap_or(1).max(1);
    let out = crate::roll::roll_expr(
        cmd,
        ctx,
        ctx.actor,
        &wp.to_string(),
        diff.unwrap_or(7),
        false,
        "resist frenzy",
    )?;
    let verdict = if out.is_botch() {
        "The Beast takes you utterly — you fall into frenzy (or the Thrall of the Wyrm)."
    } else if out.successes() > 0 {
        "You hold the Beast in check."
    } else {
        "You fail to master the Beast; the Storyteller adjudicates the frenzy."
    };
    ctx.out.sys_to_actor(ctx.actor, "wod", verdict);
    Ok(())
}

/// `+gnosis` / `+gnosis/spend <n>` / `+gnosis/gain <n>` (Garou or Kinfolk).
fn gnosis_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_garou_or_kin(cmd, ctx)?;
    let perm = doc.number("gnosis").unwrap_or(0) as i32;
    let cur = doc.number("gnosis_temp").unwrap_or(0) as i32;
    if ctx.switches.has("spend") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Spend how much Gnosis?"));
        }
        if n > cur {
            return Err(CmdError::msg(format!(
                "You have only {cur} Gnosis; you cannot spend {n}."
            )));
        }
        set_pool(cmd, ctx, &mut doc, "gnosis_temp", cur - n)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!(
                "You spend {n} Gnosis (a Gift, stepping sideways, a fetish…). Gnosis {}/{perm}.",
                cur - n
            ),
        );
        return Ok(());
    }
    if ctx.switches.has("gain") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Gain how much Gnosis?"));
        }
        // Temporary Gnosis recovers only up to the permanent rating; raising
        // the permanent rating is an XP matter (+xp/spend gnosis=…), never free.
        if cur >= perm {
            return Err(CmdError::msg(format!(
                "Your Gnosis pool is already full ({perm}). Raise permanent Gnosis with \
                 +xp/spend gnosis=<n>."
            )));
        }
        let next = (cur + n).min(perm);
        set_pool(cmd, ctx, &mut doc, "gnosis_temp", next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!("Your Gnosis rises to {next}/{perm}."),
        );
        return Ok(());
    }
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Gnosis: {cur}/{perm} (temporary/permanent)."),
    );
    Ok(())
}

/// `+renown` (show) / `+renown/gain <type>=<n>` (add temporary renown).
fn renown_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("gain") {
        return renown_gain(cmd, ctx);
    }
    let doc = require_garou(cmd, ctx)?;
    let read = |cat: &str| doc.number(cat).unwrap_or(0);
    ctx.out.sys_to_actor(ctx.actor, "wod", "Renown:");
    for (label, perm, temp) in [
        ("Glory", "glory_perm", "glory_temp"),
        ("Honor", "honor_perm", "honor_temp"),
        ("Wisdom", "wisdom_perm", "wisdom_temp"),
    ] {
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!(
                "  {label:<7} {} permanent, {} temporary",
                read(perm),
                read(temp)
            ),
        );
    }
    let (rank, name) = rank_for(permanent_renown(&doc));
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("Rank {rank}: {name}."));
    Ok(())
}

/// `+renown/gain <type>=<n>` — add temporary Renown of a category (Storyteller
/// still adjudicates converting temporary into permanent).
fn renown_gain(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_garou(cmd, ctx)?;
    let cat = ctx.args.lhs().trim().to_ascii_lowercase();
    let field = match cat.as_str() {
        "glory" => "glory_temp",
        "honor" => "honor_temp",
        "wisdom" => "wisdom_temp",
        _ => {
            return Err(CmdError::msg(
                "Usage: +renown/gain <glory|honor|wisdom>=<n>.",
            ));
        }
    };
    let n: i32 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| CmdError::msg("Usage: +renown/gain <glory|honor|wisdom>=<n> (n > 0)."))?;
    let cur = doc.number(field).unwrap_or(0) as i32;
    let next = (cur + n).min(10);
    set_pool(cmd, ctx, &mut doc, field, next)?;
    let mut msg = format!("Temporary {cat} Renown is now {next}/10.");
    if next >= 10 {
        msg.push_str(
            " (Ten temporary points can convert to a permanent point — see a Storyteller.)",
        );
    }
    ctx.out.sys_to_actor(ctx.actor, "wod", msg);
    Ok(())
}

/// `+gifts` / `+gift` (list) / `+gift/roll <gift>=<pool> [vs <diff>]` (Garou or
/// Kinfolk — Kinfolk with Gnosis may know a level-1 Gift).
fn gift_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_garou_or_kin(cmd, ctx)?;
    if ctx.switches.has("roll") {
        return gift_roll(cmd, ctx, &doc);
    }
    ctx.out.sys_to_actor(ctx.actor, "wod", "Your Gifts:");
    let mut any = false;
    if let Some(m) = doc.map("gifts") {
        let mut entries: Vec<(&String, i64)> = m
            .iter()
            .filter_map(|(k, v)| v.as_i64().map(|n| (k, n)))
            .filter(|(_, n)| *n > 0)
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (k, n) in entries {
            any = true;
            ctx.out
                .sys_to_actor(ctx.actor, "wod", format!("  {k} (Level {n})"));
        }
    }
    if !any {
        ctx.out.sys_to_actor(ctx.actor, "wod", "  (none)");
    }
    Ok(())
}

/// `+gift/roll <gift>=<attribute>+<ability> [vs <diff>]` — build and roll the
/// pool the Gift names.
fn gift_roll(cmd: &WodCmd, ctx: &mut Ctx<'_>, doc: &SheetDoc) -> Result<(), CmdError> {
    let (gift, expr, diff) = {
        let raw = ctx.args.raw();
        let (gift, rest) = raw.split_once('=').ok_or_else(|| {
            CmdError::msg("Usage: +gift/roll <gift>=<attribute>+<ability> [vs <diff>].")
        })?;
        let gift = gift.trim().to_string();
        let (expr, diff) = split_vs(rest);
        (gift, expr, diff)
    };
    if gift.is_empty() {
        return Err(CmdError::msg(
            "Name the Gift: +gift/roll <gift>=<attribute>+<ability> [vs <diff>].",
        ));
    }
    if !has_gift(doc, &gift) {
        return Err(CmdError::msg(format!("You do not know the Gift {gift:?}.")));
    }
    if expr.trim().is_empty() {
        return Err(CmdError::msg(
            "Give the Gift's pool, e.g. +gift/roll razor claws=dexterity+brawl vs 6.",
        ));
    }
    crate::roll::roll_expr(cmd, ctx, ctx.actor, &expr, diff.unwrap_or(6), true, &gift)?;
    Ok(())
}

/// `+shift <form>` — change form and report the stat deltas.
fn shift_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut doc = require_garou(cmd, ctx)?;
    let form = ctx.args.raw().trim().to_ascii_lowercase();
    if form.is_empty() {
        let cur = doc.text("current_form").unwrap_or("homid");
        return Err(CmdError::msg(format!(
            "You are in {cur} form. Shift with +shift <{}>.",
            FORMS.join("|")
        )));
    }
    if !is_form(&form) {
        return Err(CmdError::msg(format!(
            "Unknown form {form:?}. Choose one of: {}.",
            FORMS.join(", ")
        )));
    }
    let store = cmd.store_for_doc(&doc);
    store
        .set_field(&mut doc, "current_form", serde_json::json!(form))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_by(ctx.w, ctx.actor, &doc, ctx.actor)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    let mut msg = format!(
        "You shift into {form} form ({}).",
        form_delta_summary(&form)
    );
    if form.as_str() == "crinos" {
        msg.push_str(" The Delirium grips any humans who witness this shape.");
    }
    ctx.out.sys_to_actor(ctx.actor, "wod", msg);
    Ok(())
}

/// `+rank` — show Rank derived from permanent Renown.
fn rank_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_garou(cmd, ctx)?;
    let total = permanent_renown(&doc);
    let (rank, name) = rank_for(total);
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Rank {rank}: {name} ({total} permanent Renown)."),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

/// Whether the sheet knows a Gift by name (case-insensitive, level > 0).
fn has_gift(doc: &SheetDoc, gift: &str) -> bool {
    doc.map("gifts").is_some_and(|m| {
        m.iter()
            .any(|(k, v)| k.eq_ignore_ascii_case(gift) && v.as_i64().unwrap_or(0) > 0)
    })
}

/// Validate + persist a splat pool scalar (Rage / Gnosis / a Renown field).
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
        .put_by(ctx.w, ctx.actor, doc, ctx.actor)
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

fn fmt_errs(errs: &[mush_ruleset::SheetError]) -> String {
    errs.iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The Werewolf splat behind an `Arc<dyn Splat>` (the resolver entry point).
#[must_use]
pub(crate) fn template() -> Arc<dyn Splat> {
    Arc::new(Werewolf)
}

/// Register the Garou command set.
pub(crate) fn register_commands(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+rage".into(),
            switches: vec!["spend".into(), "gain".into(), "frenzy".into()],
            shape: ArgShape::Raw,
            help: "+rage  --  Show your Rage pool.\n\
                   +rage/spend <n>  --  Spend Rage (1 Rage = 1 extra action).\n\
                   +rage/gain <n>  --  Recover Rage (up to 10).\n\
                   +rage/frenzy [vs <diff>]  --  Roll Willpower to resist frenzy."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| rage_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+gnosis".into(),
            switches: vec!["spend".into(), "gain".into()],
            shape: ArgShape::Raw,
            help: "+gnosis  --  Show your Gnosis pool.\n\
                   +gnosis/spend <n>  --  Spend Gnosis (Gifts, stepping sideways, fetishes).\n\
                   +gnosis/gain <n>  --  Recover Gnosis (up to 10)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| gnosis_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+renown".into(),
            switches: vec!["gain".into()],
            shape: ArgShape::EqPair,
            help: "+renown  --  Show your Glory/Honor/Wisdom and Rank.\n\
                   +renown/gain <glory|honor|wisdom>=<n>  --  Log temporary Renown."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| renown_cmd(&cmd, ctx))
        },
    )?;
    let gift_help = "+gifts  --  List the Gifts you know.\n\
        +gift/roll <gift>=<attribute>+<ability> [vs <diff>]  --  Build and roll the \
        pool a Gift names (e.g. +gift/roll razor claws=dexterity+brawl vs 6).";
    for name in ["+gift", "+gifts"] {
        reg.command(
            CommandSpec {
                name: name.into(),
                switches: vec!["roll".into()],
                shape: ArgShape::Raw,
                help: gift_help.into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| gift_cmd(&cmd, ctx))
            },
        )?;
    }
    reg.command(
        CommandSpec {
            name: "+shift".into(),
            shape: ArgShape::Raw,
            help: "+shift <homid|glabro|crinos|hispo|lupus>  --  Change form; reports the \
                   form's stat deltas (Physical bonuses apply to your rolls while shifted)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| shift_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+rank".into(),
            shape: ArgShape::Raw,
            help: "+rank  --  Show your Rank (derived from permanent Renown).".into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| rank_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splats::{Dispatch, MortalSplat};

    /// A dispatch over mortal + werewolf built directly (no resolver arm needed).
    fn dispatch() -> Dispatch {
        let splats: Vec<Arc<dyn Splat>> = vec![Arc::new(MortalSplat), Arc::new(Werewolf)];
        Dispatch::build("wod", splats)
    }

    fn werewolf_doc() -> SheetDoc {
        let d = dispatch();
        let mut doc = SheetDoc::new_default(&d.schemas.schema_for_id("werewolf"));
        doc.set_text("template", "werewolf");
        doc
    }

    #[test]
    fn schema_has_garou_sections_and_not_other_splats() {
        let d = dispatch();
        let w = d.schemas.schema_for_id("werewolf");
        for id in [
            "rage",
            "gnosis",
            "current_form",
            "glory_perm",
            "gifts",
            "breed",
            "auspice",
            "tribe",
        ] {
            assert!(w.field(id).is_some(), "werewolf schema missing {id}");
        }
        // Not a vampire / mage.
        assert!(w.field("disciplines").is_none());
        assert!(w.field("bloodpool").is_none());
        assert!(w.field("spheres").is_none());
        // A mortal carries none of the Garou fields.
        let m = d.schemas.schema_for_id("mortal");
        assert!(m.field("rage").is_none());
        assert!(m.field("gnosis").is_none());
        assert!(m.field("gifts").is_none());
        // Both default docs validate.
        assert!(mush_ruleset::validate(&w, &w.default_data()).is_ok());
        assert!(mush_ruleset::validate(&m, &m.default_data()).is_ok());
    }

    #[test]
    fn breed_gnosis_and_auspice_rage_tables() {
        assert_eq!(breed_def("homid").unwrap().gnosis, 1);
        assert_eq!(breed_def("metis").unwrap().gnosis, 3);
        assert_eq!(breed_def("lupus").unwrap().gnosis, 5);
        assert_eq!(auspice_def("ragabash").unwrap().rage, 1);
        assert_eq!(auspice_def("philodox").unwrap().rage, 3);
        assert_eq!(auspice_def("ahroun").unwrap().rage, 5);
        assert!(!breed_def("metis").unwrap().flaw.is_empty());
        assert!(breed_def("homid").unwrap().flaw.is_empty());
    }

    #[test]
    fn tribe_table_willpower_matches_w20() {
        assert_eq!(TRIBES.len(), 13);
        // W20 tribal starting Willpower: Silver Fangs are 3, like most tribes;
        // the ceiling is 4 (Bone Gnawers / Children of Gaia / Stargazers /
        // Wendigo). No tribe starts at 5.
        assert_eq!(tribe_def("silver_fangs").unwrap().willpower, 3);
        assert_eq!(tribe_def("black_furies").unwrap().willpower, 3);
        assert_eq!(tribe_def("bone_gnawers").unwrap().willpower, 4);
        assert_eq!(tribe_def("wendigo").unwrap().willpower, 4);
        assert!(
            TRIBES.iter().all(|t| (3..=4).contains(&t.willpower)),
            "every tribe's starting Willpower is 3 or 4"
        );
        assert!(!tribe_def("get_of_fenris").unwrap().ban.is_empty());
    }

    #[test]
    fn finalize_derives_the_starting_pools() {
        let mut doc = werewolf_doc();
        doc.set_text("breed", "lupus"); // Gnosis 5
        doc.set_text("auspice", "ahroun"); // Rage 5, Glory 3
        doc.set_text("tribe", "silver_fangs"); // Willpower 3
        Werewolf.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(doc.number("rage"), Some(5), "Ahroun permanent Rage");
        assert_eq!(doc.number("rage_temp"), Some(5), "Rage pool starts full");
        assert_eq!(doc.number("gnosis"), Some(5), "Lupus permanent Gnosis");
        assert_eq!(
            doc.number("gnosis_temp"),
            Some(5),
            "Gnosis pool starts full"
        );
        assert_eq!(doc.number("willpower"), Some(3), "Silver Fang Willpower");
        assert_eq!(doc.number("willpower_temp"), Some(3));
        assert_eq!(doc.number("glory_perm"), Some(3));
        assert_eq!(doc.number("rank"), Some(1), "3 Renown = Cliath");
        assert_eq!(doc.text("current_form"), Some("homid"));
        assert!(doc.text("ban").unwrap().to_lowercase().contains("honour"));
        // One gift from each source (Lupus/Ahroun/Silver Fangs first gifts).
        let gifts = doc.map("gifts").unwrap();
        assert!(gifts.contains_key("Hare's Leap"), "breed gift granted");
        assert!(gifts.contains_key("Falling Touch"), "auspice gift granted");
        assert!(gifts.contains_key("Falcon's Grasp"), "tribe gift granted");
        // The derived doc validates on the werewolf schema.
        let d = dispatch();
        assert!(mush_ruleset::validate(&d.schemas.schema_for_id("werewolf"), &doc.data).is_ok());
    }

    #[test]
    fn willpower_is_the_tribe_value_not_a_courage_floor() {
        // A Garou who pumped Courage (so the base finalize left Willpower high)
        // still ends at the fixed tribal Willpower — Courage does not inflate it.
        let mut doc = werewolf_doc();
        doc.set_text("breed", "homid");
        doc.set_text("auspice", "ragabash");
        doc.set_text("tribe", "black_furies"); // Willpower 3
        doc.set_number("willpower", 5); // as if Courage 5 had floored it
        Werewolf.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(doc.number("willpower"), Some(3), "WP is the tribe value");
        assert_eq!(doc.number("willpower_temp"), Some(3));
        // And Willpower is chargen-locked so +stat cannot inflate it either.
        assert!(Werewolf.chargen_locked_fields().contains(&"willpower"));
    }

    #[test]
    fn metis_finalize_records_the_birth_flaw() {
        let mut doc = werewolf_doc();
        doc.set_text("breed", "metis");
        doc.set_text("auspice", "theurge");
        doc.set_text("tribe", "stargazers");
        Werewolf.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(doc.number("gnosis"), Some(3));
        assert!(
            doc.text("deformity")
                .unwrap()
                .to_lowercase()
                .contains("deform")
        );
    }

    #[test]
    fn rank_thresholds() {
        assert_eq!(rank_for(0).0, 0);
        assert_eq!(rank_for(2).0, 0);
        assert_eq!(rank_for(3).0, 1);
        assert_eq!(rank_for(7).0, 2);
        assert_eq!(rank_for(13).0, 3);
        assert_eq!(rank_for(19).0, 4);
        assert_eq!(rank_for(26).0, 5);
        assert_eq!(rank_for(30).1, "Elder");
    }

    #[test]
    fn crinos_form_boosts_a_strength_pool_only_for_physical_traits() {
        let mut doc = werewolf_doc();
        doc.set_text("current_form", "crinos");
        // Strength pool: +4 in Crinos.
        assert_eq!(current_form_pool_delta(&doc, &["strength".to_string()]), 4);
        // Strength + Stamina: +4 +3 = +7.
        assert_eq!(
            current_form_pool_delta(&doc, &["strength".to_string(), "stamina".to_string()]),
            7
        );
        // A non-physical pool (Brawl only) is unshifted.
        assert_eq!(current_form_pool_delta(&doc, &["brawl".to_string()]), 0);
        // Manipulation is penalised in Crinos.
        assert_eq!(
            current_form_pool_delta(&doc, &["manipulation".to_string()]),
            -3
        );
        // Appearance drops to 0 (remove every dot) in the war form.
        doc.set_number("appearance", 3);
        assert_eq!(
            current_form_pool_delta(&doc, &["appearance".to_string()]),
            -3
        );
        // Homid = no change.
        doc.set_text("current_form", "homid");
        assert_eq!(current_form_pool_delta(&doc, &["strength".to_string()]), 0);
        // A non-Garou sheet is never shifted.
        let mut mortal = doc.clone();
        mortal.set_text("template", "mortal");
        mortal.set_text("current_form", "crinos");
        assert_eq!(
            current_form_pool_delta(&mortal, &["strength".to_string()]),
            0
        );
    }

    #[test]
    fn gift_and_pool_xp_costs() {
        // Gift level 1 = 3, level 2 = 6; 0→2 = 9.
        assert_eq!(gift_raise_cost(0, 1), 3);
        assert_eq!(gift_raise_cost(1, 2), 6);
        assert_eq!(gift_raise_cost(0, 2), 9);
        // Rage ×1: 3→4 = 3. Gnosis ×2: 3→4 = 6.
        assert_eq!(
            Werewolf.xp_cost(&WodConfig::default(), &werewolf_doc(), "rage", None, 3, 4),
            3
        );
        assert_eq!(
            Werewolf.xp_cost(&WodConfig::default(), &werewolf_doc(), "gnosis", None, 3, 4),
            6
        );
        // The splat claims gifts/rage/gnosis for XP, nothing else.
        assert_eq!(Werewolf.xp_max("gifts", Some("Razor Claws")), Some(5));
        assert_eq!(Werewolf.xp_max("rage", None), Some(10));
        assert_eq!(Werewolf.xp_max("gnosis", None), Some(10));
        assert_eq!(Werewolf.xp_max("strength", None), None);
    }

    #[test]
    fn rage_and_gnosis_are_split_perm_and_temp() {
        // The spendable pool is a separate field from the permanent (XP-raised)
        // rating, so +rage/gain can never inflate the permanent trait for free.
        let d = dispatch();
        let w = d.schemas.schema_for_id("werewolf");
        for f in ["rage", "rage_temp", "gnosis", "gnosis_temp"] {
            assert!(w.field(f).is_some(), "werewolf schema missing {f}");
        }
        // The temporary pools are chargen-locked (derived at finalize), and the
        // permanent traits remain the XP targets.
        let locked = Werewolf.chargen_locked_fields();
        assert!(locked.contains(&"rage_temp"));
        assert!(locked.contains(&"gnosis_temp"));
        assert_eq!(Werewolf.xp_max("rage", None), Some(10));
        assert_eq!(Werewolf.xp_max("gnosis", None), Some(10));
        // rage_temp / gnosis_temp are pools, not XP targets.
        assert_eq!(Werewolf.xp_max("rage_temp", None), None);
        assert_eq!(Werewolf.xp_max("gnosis_temp", None), None);
    }

    #[test]
    fn on_field_changed_updates_ban_and_rank() {
        let mut doc = werewolf_doc();
        doc.set_text("tribe", "get_of_fenris");
        Werewolf.on_field_changed(&mut doc, "tribe");
        assert!(doc.text("ban").unwrap().to_lowercase().contains("courage"));
        // Staff bumps permanent Glory → Rank re-derives.
        doc.set_number("glory_perm", 7);
        Werewolf.on_field_changed(&mut doc, "glory_perm");
        assert_eq!(doc.number("rank"), Some(2), "7 Renown = Fostern");
    }
}
