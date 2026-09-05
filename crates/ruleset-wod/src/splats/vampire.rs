//! # Vampire: The Masquerade 20th Anniversary (V20) — the reference splat
//!
//! The worked implementation of the [`Splat`] contract (see the module docs in
//! [`crate::splats`]). Kindred are **sheet-tracked, pool-rolled**: Disciplines
//! are a name→dots map rolled as classic Storyteller pools, splat pools
//! (Blood) get spend/gain/show commands, and Frenzy/Humanity are roll helpers —
//! no bespoke per-power automation.
//!
//! What this module adds on top of the base oWoD sheet:
//!
//! - a **Clan** ([`CLANS`], 13 + Caitiff): three in-clan Disciplines and a
//!   weakness each;
//! - **Generation** (default 13th) driving max Blood Pool, blood spent per
//!   turn, and the trait maximum ([`generation_traits`]);
//! - a **Disciplines** map (name→0..5) with in-clan vs out-of-clan XP costs;
//! - a **Blood Pool** with `+blood` spend/gain/heal/buff;
//! - Virtues → **Humanity** (or a Path of Enlightenment) and starting
//!   Willpower;
//! - commands `+disc`, `+blood`, `+frenzy`, `+generation`, `+humanity`, all
//!   self-gating on the actor being Kindred.

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
use crate::perception::{ActPerceptibility, SenseChannel, default_perceptibility};
use crate::schema::Template;
use crate::splats::ghoul;
use crate::splats::{ChargenContribution, Splat, SplatPool, SplatPoolSource, is_template};

/// The canonical V20 Discipline list (others are allowed on the map).
pub const DISCIPLINES: &[&str] = &[
    "Animalism",
    "Auspex",
    "Celerity",
    "Chimerstry",
    "Dementation",
    "Dominate",
    "Fortitude",
    "Necromancy",
    "Obfuscate",
    "Obtenebration",
    "Potence",
    "Presence",
    "Protean",
    "Quietus",
    "Serpentis",
    "Thaumaturgy",
    "Vicissitude",
];

/// One clan's crib entry: id, label, its three in-clan Disciplines (empty for
/// Caitiff), and its weakness text.
pub struct ClanDef {
    /// The `clan` enum id (snake_case).
    pub id: &'static str,
    /// Human label.
    pub label: &'static str,
    /// In-clan Disciplines (cost less in XP; 3 free dots at chargen).
    pub disciplines: &'static [&'static str],
    /// The clan weakness (derived onto the `weakness` sheet field).
    pub weakness: &'static str,
}

/// The 13 clans plus Caitiff (crib data table).
pub const CLANS: &[ClanDef] = &[
    ClanDef {
        id: "assamite",
        label: "Assamite",
        disciplines: &["Celerity", "Obfuscate", "Quietus"],
        weakness: "Addicted to Kindred vitae; the blood of others is one difficulty harder to resist drinking.",
    },
    ClanDef {
        id: "brujah",
        label: "Brujah",
        disciplines: &["Celerity", "Potence", "Presence"],
        weakness: "Fiery tempered: +2 difficulty (or -2 dice) on rolls to resist frenzy.",
    },
    ClanDef {
        id: "followers_of_set",
        label: "Followers of Set",
        disciplines: &["Obfuscate", "Presence", "Serpentis"],
        weakness: "Recoil from sunlight and bright light, suffering double the usual penalties.",
    },
    ClanDef {
        id: "gangrel",
        label: "Gangrel",
        disciplines: &["Animalism", "Fortitude", "Protean"],
        weakness: "Each frenzy leaves an animal feature until it is worked off.",
    },
    ClanDef {
        id: "giovanni",
        label: "Giovanni",
        disciplines: &["Dominate", "Necromancy", "Potence"],
        weakness: "Their kiss brings agony, not ecstasy, inflicting aggravated damage.",
    },
    ClanDef {
        id: "lasombra",
        label: "Lasombra",
        disciplines: &["Dominate", "Obtenebration", "Potence"],
        weakness: "Cast no reflection and are ill at ease before mirrors and cameras.",
    },
    ClanDef {
        id: "malkavian",
        label: "Malkavian",
        disciplines: &["Auspex", "Dementation", "Obfuscate"],
        weakness: "Bound to at least one incurable derangement.",
    },
    ClanDef {
        id: "nosferatu",
        label: "Nosferatu",
        disciplines: &["Animalism", "Obfuscate", "Potence"],
        weakness: "Monstrous: Appearance 0, and social rolls that rely on looks fail.",
    },
    ClanDef {
        id: "ravnos",
        label: "Ravnos",
        disciplines: &["Animalism", "Chimerstry", "Fortitude"],
        weakness: "Bound to a particular vice they must indulge (roll to resist).",
    },
    ClanDef {
        id: "toreador",
        label: "Toreador",
        disciplines: &["Auspex", "Celerity", "Presence"],
        weakness: "Entranced by beauty: risk being transfixed by a lovely sight.",
    },
    ClanDef {
        id: "tremere",
        label: "Tremere",
        disciplines: &["Auspex", "Dominate", "Thaumaturgy"],
        weakness: "Partly blood-bound to the Council of Seven by their initiation.",
    },
    ClanDef {
        id: "tzimisce",
        label: "Tzimisce",
        disciplines: &["Animalism", "Auspex", "Vicissitude"],
        weakness: "Must rest surrounded by earth from a place important to them.",
    },
    ClanDef {
        id: "ventrue",
        label: "Ventrue",
        disciplines: &["Dominate", "Fortitude", "Presence"],
        weakness: "Refined palate: can feed only from one particular kind of prey.",
    },
    ClanDef {
        id: "caitiff",
        label: "Caitiff",
        disciplines: &[],
        weakness: "Clanless: no clan Disciplines and no reliable standing among the Kindred.",
    },
];

/// The clan definition for an id (case-insensitive).
#[must_use]
pub fn clan_def(id: &str) -> Option<&'static ClanDef> {
    CLANS.iter().find(|c| c.id.eq_ignore_ascii_case(id))
}

/// The clan enum options (the ids).
fn clan_ids() -> Vec<SmolStr> {
    CLANS.iter().map(|c| SmolStr::new(c.id)).collect()
}

/// The clan recorded on a sheet, if any.
#[must_use]
pub fn clan_of(doc: &SheetDoc) -> Option<&'static ClanDef> {
    doc.text("clan").and_then(clan_def)
}

/// A sheet's dots in one Discipline (case-insensitive), `0` if absent.
///
/// The single reader for a Discipline rating off the `disciplines` map — used
/// by the M11 perception vertical (Obfuscate as concealment strength, Auspex
/// as the sense that pierces it) so a vampire and a ghoul, whose sheets carry
/// the identical map, read the same way.
#[must_use]
pub fn discipline_rating(doc: &SheetDoc, discipline: &str) -> i32 {
    doc.map("disciplines")
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(discipline))
                .and_then(|(_, v)| v.as_i64())
        })
        .unwrap_or(0) as i32
}

/// The per-Discipline perceptibility table (M14): how an onlooker notices a
/// Discipline in use, and the line a successful witness perceives. Subtle
/// mental/social Disciplines travel the **Supernatural** channel and are gated
/// behind **Auspex** (only the preternaturally aware notice at all); overt
/// physical Disciplines are **Visual** and widely noticed (low difficulty). Any
/// canonical Discipline without a bespoke row falls back to the moderate default.
///
/// Returns `None` for a name that is not a canonical Discipline — the signal the
/// NPC power path ([`power_perceptibility_for_pool`](crate::perception::power_perceptibility_for_pool))
/// uses to tell a Discipline pool from a mundane one.
#[must_use]
pub(crate) fn discipline_perceptibility(name: &str, actor: &str) -> Option<ActPerceptibility> {
    if !DISCIPLINES.iter().any(|d| d.eq_ignore_ascii_case(name)) {
        return None;
    }
    let sup = SenseChannel::Supernatural;
    let vis = SenseChannel::Visual;
    let eq = |d: &str| name.eq_ignore_ascii_case(d);
    let perc = if eq("Dominate") {
        ActPerceptibility::new(
            sup,
            8,
            Some("Auspex"),
            format!("You sense an unseen pressure as {actor} bends another's will."),
        )
    } else if eq("Presence") {
        ActPerceptibility::new(
            sup,
            7,
            Some("Auspex"),
            format!("You feel an unnatural magnetism radiating from {actor}."),
        )
    } else if eq("Dementation") {
        ActPerceptibility::new(
            sup,
            8,
            Some("Auspex"),
            format!("A maddening whisper seems to trail from {actor}."),
        )
    } else if eq("Obfuscate") {
        ActPerceptibility::new(
            sup,
            9,
            Some("Auspex"),
            format!("For an instant you glimpse {actor} where your eye insists no one stands."),
        )
    } else if eq("Auspex") {
        ActPerceptibility::new(
            sup,
            8,
            Some("Auspex"),
            format!("You feel {actor}'s senses brush past the veil."),
        )
    } else if eq("Chimerstry") {
        ActPerceptibility::new(
            sup,
            8,
            Some("Auspex"),
            format!("Something about {actor}'s handiwork rings false to your sharper sense."),
        )
    } else if eq("Celerity") {
        ActPerceptibility::new(
            vis,
            5,
            None,
            format!("You see {actor} blur into impossible speed."),
        )
    } else if eq("Potence") {
        ActPerceptibility::new(
            vis,
            5,
            None,
            format!("You watch {actor} strike with impossible strength."),
        )
    } else if eq("Protean") {
        ActPerceptibility::new(
            vis,
            5,
            None,
            format!("You see {actor}'s flesh warp inhumanly."),
        )
    } else if eq("Vicissitude") {
        ActPerceptibility::new(
            vis,
            6,
            None,
            format!("You see flesh reshape at {actor}'s touch."),
        )
    } else if eq("Obtenebration") {
        ActPerceptibility::new(
            vis,
            6,
            None,
            format!("You watch shadows crawl unnaturally toward {actor}."),
        )
    } else {
        default_perceptibility(format!(
            "You notice {actor} exert some preternatural power."
        ))
    };
    Some(perc)
}

/// The perceptibility for a `+disc/roll` pool: the first Discipline the actor
/// actually has whose name appears in `expr` selects the row (shared by the
/// Kindred and ghoul `+disc/roll` paths); a pool naming no owned Discipline
/// falls back to the default.
#[must_use]
pub(crate) fn discipline_perceptibility_for_expr(
    doc: &SheetDoc,
    expr: &str,
    actor: &str,
) -> ActPerceptibility {
    let lower = expr.to_ascii_lowercase();
    let name = doc.map("disciplines").and_then(|m| {
        m.iter()
            .filter(|(_, v)| v.as_i64().unwrap_or(0) > 0)
            .map(|(k, _)| k.clone())
            .find(|k| lower.contains(&k.to_ascii_lowercase()))
    });
    match name {
        Some(n) => discipline_perceptibility(&n, actor).unwrap_or_else(|| {
            default_perceptibility(format!(
                "You notice {actor} exert some preternatural power."
            ))
        }),
        None => default_perceptibility(format!(
            "You notice {actor} exert some preternatural power."
        )),
    }
}

/// Whether `discipline` is in-clan for the sheet's clan (Caitiff: never).
#[must_use]
pub fn is_in_clan(doc: &SheetDoc, discipline: &str) -> bool {
    clan_of(doc).is_some_and(|c| {
        c.disciplines
            .iter()
            .any(|d| d.eq_ignore_ascii_case(discipline))
    })
}

/// Generation-derived traits (crib table): max Blood Pool, blood spendable per
/// turn, and the trait maximum.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GenTraits {
    /// Maximum Blood Pool.
    pub max_blood: i32,
    /// Blood points spendable per turn.
    pub per_turn: i32,
    /// Maximum rating a trait may reach.
    pub trait_max: i32,
}

/// The V20 generation table for `generation` (clamped to the playable 4..=15;
/// 13th and thinner share the 10/1/5 row).
#[must_use]
pub fn generation_traits(generation: i32) -> GenTraits {
    let (max_blood, per_turn, trait_max) = match generation.clamp(4, 15) {
        4 => (50, 10, 9),
        5 => (40, 8, 8),
        6 => (30, 6, 7),
        7 => (20, 4, 6),
        8 => (15, 3, 5),
        9 => (14, 2, 5),
        10 => (13, 1, 5),
        11 => (12, 1, 5),
        12 => (11, 1, 5),
        _ => (10, 1, 5),
    };
    GenTraits {
        max_blood,
        per_turn,
        trait_max,
    }
}

/// Whether a sheet follows a Path of Enlightenment (a non-empty `path` other
/// than "Humanity").
#[must_use]
fn on_path(doc: &SheetDoc) -> bool {
    let p = doc.text("path").unwrap_or("Humanity").trim();
    !p.is_empty() && !p.eq_ignore_ascii_case("Humanity")
}

/// The two active morality Virtues for a sheet: Conscience/Self-Control on
/// Humanity, or Conviction/Instinct on a Path. (Courage is always the third.)
#[must_use]
fn active_virtues(doc: &SheetDoc) -> (&'static str, &'static str) {
    if on_path(doc) {
        ("conviction", "instinct")
    } else {
        ("conscience", "self_control")
    }
}

/// The XP cost to raise a Discipline `from`→`to`: a new Discipline (0→1) costs
/// 10; further dots cost current × 5 (in-clan), × 6 (Caitiff), or × 7
/// (out-of-clan).
#[must_use]
pub fn discipline_raise_cost(doc: &SheetDoc, discipline: &str, from: i32, to: i32) -> i32 {
    let mult = if clan_of(doc).is_none_or(|c| c.disciplines.is_empty()) {
        6 // Caitiff / no clan
    } else if is_in_clan(doc, discipline) {
        5
    } else {
        7
    };
    (from.max(0)..to)
        .map(|r| if r <= 0 { 10 } else { r * mult })
        .sum()
}

// ----------------------------------------------------------------------
// The Vampire splat
// ----------------------------------------------------------------------

/// The V20 splat.
#[derive(Debug, Default, Clone, Copy)]
pub struct Vampire;

impl Template for Vampire {
    fn id(&self) -> &'static str {
        "vampire"
    }

    fn display_name(&self) -> &str {
        "Vampire"
    }

    fn extend_schema(&self, b: SheetSchemaBuilder) -> Result<SheetSchemaBuilder, SchemaError> {
        // Path Virtues live in the base Virtues section.
        let b = b.extend_section(
            "virtues",
            [
                FieldDef::dots("conviction", "Conviction", 0, 5).category("virtue"),
                FieldDef::dots("instinct", "Instinct", 0, 5).category("virtue"),
            ],
        )?;
        // Kindred scalars.
        let kindred = SheetSection::new("kindred", "Kindred", SectionLayout::List)
            .field(
                FieldDef::enumeration("clan", "Clan", clan_ids())
                    .with_default(serde_json::json!("caitiff")),
            )
            .field(
                FieldDef::number("generation", "Generation", 4, 15)
                    .with_default(serde_json::json!(13)),
            )
            .field(
                FieldDef::number("bloodpool", "Blood Pool", 0, 50)
                    .vital(mush_proto::VitalRole::Pool),
            )
            .field(
                FieldDef::number("humanity", "Humanity / Path", 0, 10)
                    .with_default(serde_json::json!(7)),
            )
            .field(
                FieldDef::text("path", "Path of Enlightenment", 40)
                    .with_default(serde_json::json!("Humanity")),
            )
            .field(FieldDef::text("weakness", "Clan Weakness", 200));
        let b = b.section(kindred)?;
        // Disciplines map.
        let disc = SheetSection::new("disciplines", "Disciplines", SectionLayout::Map).field(
            FieldDef::map(
                "disciplines",
                "Disciplines",
                FieldType::Dots { min: 0, max: 5 },
            ),
        );
        b.section(disc)
    }
}

impl Splat for Vampire {
    fn register_commands(&self, reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
        register_commands(reg, cmd)
    }

    fn chargen_contribution(&self, _config: &WodConfig, doc: &SheetDoc) -> ChargenContribution {
        let (v1, v2) = active_virtues(doc);
        ChargenContribution {
            pools: vec![
                SplatPool {
                    id: SmolStr::new("disciplines"),
                    label: "Disciplines".to_string(),
                    total: 3,
                    source: SplatPoolSource::MapDots(SmolStr::new("disciplines")),
                    // Simplified flat in-clan discipline freebie (crib note).
                    freebie_cost: 7,
                },
                SplatPool {
                    id: SmolStr::new("virtues"),
                    label: "Virtues".to_string(),
                    total: 7,
                    source: SplatPoolSource::ScalarDotsAboveOne(vec![
                        SmolStr::new(v1),
                        SmolStr::new(v2),
                        SmolStr::new("courage"),
                    ]),
                    freebie_cost: 2,
                },
            ],
            // The Virtue pool owns the base Virtues, so base freebie accounting
            // must not also charge them.
            freebie_exempt: vec![TraitKind::Virtue],
        }
    }

    fn finalize_starting(&self, _config: &WodConfig, doc: &mut SheetDoc) {
        // Generation: the Generation background buys a lower one (0 dots = 13th,
        // 5 = 8th); take the lower of that and any directly-set generation.
        let gen_bg = background_dots(doc, "Generation");
        let gen_field = doc.number("generation").unwrap_or(13) as i32;
        let generation = (13 - gen_bg).min(gen_field).clamp(4, 15);
        doc.set_number("generation", i64::from(generation));

        // Blood Pool starts full.
        let gt = generation_traits(generation);
        doc.set_number("bloodpool", i64::from(gt.max_blood));

        // Humanity / Path rating = the two active morality virtues.
        let (v1, v2) = active_virtues(doc);
        let rating = (doc.number(v1).unwrap_or(1) + doc.number(v2).unwrap_or(1)).clamp(0, 10);
        doc.set_number("humanity", rating);

        // Weakness text from the clan.
        if let Some(c) = clan_of(doc) {
            doc.set_text("weakness", c.weakness);
        }
        // Willpower already = Courage (config baseline) from the base finalize.
    }

    fn chargen_locked_fields(&self) -> &'static [&'static str] {
        // `generation` is derived from the Generation background at finalize
        // (13 − dots); it must NOT be a free `+stat` set, or a player could type
        // `+stat generation=4` and pick up a 4th-gen Blood Pool of 50 for zero
        // points (it sits in no budget ledger). Staff `@sheet/set` still edits it.
        &["bloodpool", "humanity", "weakness", "generation"]
    }

    fn wizard_splat_fields(&self) -> &'static [&'static str] {
        &["clan", "path"]
    }

    fn wizard_pool_field(&self) -> Option<&'static str> {
        Some("disciplines")
    }

    fn wizard_set_pool(
        &self,
        cmd: &WodCmd,
        ctx: &mut Ctx<'_>,
        name: &str,
        rating: i32,
    ) -> Result<(), CmdError> {
        // Disciplines are not chargen-locked, so the ordinary +stat map path
        // handles them (in-clan / out-of-clan is a costing concern, not a
        // set-time one). Overflow beyond the 3-dot pool costs freebies.
        crate::chargen_cmds::apply_stat(cmd, ctx, "disciplines", Some(name), &rating.to_string())
    }

    fn on_field_changed(&self, doc: &mut SheetDoc, field: &str) {
        if field.eq_ignore_ascii_case("clan") {
            let text = clan_of(doc).map_or("", |c| c.weakness);
            doc.set_text("weakness", text);
        }
    }

    fn xp_max(&self, field: &str, key: Option<&str>) -> Option<i32> {
        (field.eq_ignore_ascii_case("disciplines") && key.is_some()).then_some(5)
    }

    fn xp_cost(
        &self,
        _config: &WodConfig,
        doc: &SheetDoc,
        _field: &str,
        key: Option<&str>,
        from: i32,
        to: i32,
    ) -> i32 {
        discipline_raise_cost(doc, key.unwrap_or(""), from, to)
    }
}

/// Total dots of a named background on a sheet.
fn background_dots(doc: &SheetDoc, name: &str) -> i32 {
    doc.map("backgrounds")
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .and_then(|(_, v)| v.as_i64())
        })
        .unwrap_or(0) as i32
}

// ----------------------------------------------------------------------
// Commands (all self-gate on the actor being Kindred)
// ----------------------------------------------------------------------

/// Refuse politely if the actor is not a vampire.
fn require_kindred(cmd: &WodCmd, ctx: &Ctx<'_>) -> Result<SheetDoc, CmdError> {
    if !is_template(cmd, ctx, "vampire") {
        return Err(CmdError::msg("You are not Kindred."));
    }
    cmd.read(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("You have no sheet."))
}

/// `+disc` / `+disciplines` / `+disc/roll <discipline>+<trait> [vs <diff>]`.
///
/// Vampire owns the `+disc` / `+disciplines` registration for the whole game;
/// a **ghoul** shares the same commands and sheet-side Disciplines map, so this
/// delegates to [`ghoul::disc_cmd`](crate::splats::ghoul) when the actor is a
/// ghoul (its in-set tagging keys off `domitor_clan`, not `clan`). See the
/// module-collision note in `ghoul.rs`.
fn disc_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if is_template(cmd, ctx, "ghoul") {
        return ghoul::disc_cmd(cmd, ctx);
    }
    let doc = require_kindred(cmd, ctx)?;
    if ctx.switches.has("roll") {
        return disc_roll(cmd, ctx, &doc);
    }
    // List the character's Disciplines.
    let name = name_of(ctx, ctx.actor);
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("{name}'s Disciplines:"));
    let mut any = false;
    if let Some(m) = doc.map("disciplines") {
        let mut entries: Vec<(&String, i64)> = m
            .iter()
            .filter_map(|(k, v)| v.as_i64().map(|n| (k, n)))
            .filter(|(_, n)| *n > 0)
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (k, n) in entries {
            any = true;
            let tag = if is_in_clan(&doc, k) {
                " (in-clan)"
            } else {
                ""
            };
            ctx.out
                .sys_to_actor(ctx.actor, "wod", format!("  {k}: {n}{tag}"));
        }
    }
    if !any {
        ctx.out.sys_to_actor(ctx.actor, "wod", "  (none)");
    }
    if let Some(c) = clan_of(&doc) {
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!("Clan {}: {}", c.label, c.weakness),
        );
    }
    Ok(())
}

/// `+disc/roll <discipline>+<trait> [vs <diff>]` — build and roll the pool.
fn disc_roll(cmd: &WodCmd, ctx: &mut Ctx<'_>, doc: &SheetDoc) -> Result<(), CmdError> {
    let (expr, diff) = split_vs(ctx.args.raw());
    if expr.trim().is_empty() {
        return Err(CmdError::msg(
            "Usage: +disc/roll <discipline>+<attribute-or-ability> [vs <diff>]",
        ));
    }
    // Require at least one referenced trait to be a Discipline the actor has.
    let has_disc = doc.map("disciplines").is_some_and(|m| {
        m.iter().any(|(k, v)| {
            v.as_i64().unwrap_or(0) > 0
                && expr.to_ascii_lowercase().contains(&k.to_ascii_lowercase())
        })
    });
    if !has_disc {
        return Err(CmdError::msg(
            "Name a Discipline you have, e.g. +disc/roll celerity+dexterity vs 6.",
        ));
    }
    // M14: a Discipline is a power — route it through the perception director so
    // each onlooker perceives-or-not (a subtle mental Discipline stays silent to
    // the unaware), instead of broadcasting the raw dice line to the room.
    let actor_name = name_of(ctx, ctx.actor);
    let perc = discipline_perceptibility_for_expr(doc, &expr, &actor_name);
    crate::roll::roll_expr(
        cmd,
        ctx,
        ctx.actor,
        &expr,
        diff.unwrap_or(6),
        true,
        "discipline",
        Some(&perc),
        false,
    )?;
    Ok(())
}

/// `+blood` / `+blood/spend <n>` / `+blood/gain <n>` / `+blood/heal [<n>]` /
/// `+blood/buff <n>`.
///
/// Vampire owns the `+blood` registration for the whole game; a **ghoul** has
/// its own flat 10-point Blood Pool (fed only by drinking), so this delegates
/// to [`ghoul::blood_cmd`](crate::splats::ghoul) when the actor is a ghoul
/// rather than run the generation-scaled Kindred table (a ghoul has no
/// `generation`). See the module-collision note in `ghoul.rs`.
fn blood_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if is_template(cmd, ctx, "ghoul") {
        return ghoul::blood_cmd(cmd, ctx);
    }
    let mut doc = require_kindred(cmd, ctx)?;
    let generation = doc.number("generation").unwrap_or(13) as i32;
    let gt = generation_traits(generation);
    let cur = doc.number("bloodpool").unwrap_or(0) as i32;

    if ctx.switches.has("heal") {
        return blood_heal(cmd, ctx, doc, cur, gt);
    }
    if ctx.switches.has("spend") || ctx.switches.has("buff") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Spend how much blood?"));
        }
        if n > cur {
            return Err(CmdError::msg(format!(
                "You have only {cur} blood; you cannot spend {n}."
            )));
        }
        let over_turn = n > gt.per_turn;
        set_blood(cmd, ctx, &mut doc, cur - n)?;
        let mut msg = if ctx.switches.has("buff") {
            format!(
                "You flood your limbs with {n} blood (raise a Physical Attribute up to your generation cap of {}). Blood {}/{}.",
                gt.trait_max,
                cur - n,
                gt.max_blood
            )
        } else {
            format!("You spend {n} blood. Blood {}/{}.", cur - n, gt.max_blood)
        };
        if over_turn {
            msg.push_str(&format!(
                " (Note: your generation lets you spend only {} per turn.)",
                gt.per_turn
            ));
        }
        ctx.out.sys_to_actor(ctx.actor, "wod", msg);
        return Ok(());
    }
    if ctx.switches.has("gain") {
        let n = arg_count(ctx, 0)?;
        if n < 1 {
            return Err(CmdError::msg("Gain how much blood?"));
        }
        let next = (cur + n).min(gt.max_blood);
        let gained = next - cur;
        set_blood(cmd, ctx, &mut doc, next)?;
        ctx.out.sys_to_actor(
            ctx.actor,
            "wod",
            format!(
                "You feed, gaining {gained} blood ({} spilled). Blood {}/{}.",
                n - gained,
                next,
                gt.max_blood
            ),
        );
        return Ok(());
    }
    // Bare: show.
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "Blood Pool: {cur}/{} (spend up to {} per turn, {}th generation).",
            gt.max_blood, gt.per_turn, generation
        ),
    );
    Ok(())
}

/// `+frenzy [vs <diff>]` (resist hunger/anger with Self-Control/Instinct) /
/// `+frenzy/fear [vs <diff>]` (resist Rötschreck with Courage).
fn frenzy_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_kindred(cmd, ctx)?;
    let (_, diff) = split_vs(ctx.args.raw());
    let (virtue, label) = if ctx.switches.has("fear") {
        ("courage", "resist Rötschreck")
    } else if on_path(&doc) {
        ("instinct", "resist frenzy")
    } else {
        ("self_control", "resist frenzy")
    };
    let out = crate::roll::roll_expr(
        cmd,
        ctx,
        ctx.actor,
        virtue,
        diff.unwrap_or(6),
        false,
        label,
        None,
        false,
    )?;
    let verdict = if out.is_botch() {
        "The Beast slips its leash — you succumb utterly."
    } else if out.successes() > 0 {
        "You master the Beast."
    } else {
        "You fail to check the Beast; the Storyteller adjudicates."
    };
    ctx.out.sys_to_actor(ctx.actor, "wod", verdict);
    Ok(())
}

/// `+generation` — show generation and its derived limits.
fn generation_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let doc = require_kindred(cmd, ctx)?;
    let generation = doc.number("generation").unwrap_or(13) as i32;
    let gt = generation_traits(generation);
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "Generation {generation}th: Blood Pool max {}, {} blood/turn, trait max {}.",
            gt.max_blood, gt.per_turn, gt.trait_max
        ),
    );
    Ok(())
}

/// `+humanity` (show) / `+humanity/set <target>=<n>` (staff adjust).
fn humanity_cmd(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("set") {
        return humanity_set(cmd, ctx);
    }
    let doc = require_kindred(cmd, ctx)?;
    let rating = doc.number("humanity").unwrap_or(0);
    let path = doc.text("path").unwrap_or("Humanity");
    ctx.out
        .sys_to_actor(ctx.actor, "wod", format!("{path}: {rating}/10."));
    Ok(())
}

/// `+humanity/set <target>=<n>` — staff adjust a character's Humanity/Path.
fn humanity_set(cmd: &WodCmd, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !perm::has_role(ctx.w, ctx.actor, Role::Royalty) {
        return Err(CmdError::msg("Permission denied."));
    }
    let target_spec = ctx.args.lhs().trim();
    let n: i64 = ctx
        .args
        .rhs()
        .and_then(|r| r.trim().parse().ok())
        .filter(|n| (0..=10).contains(n))
        .ok_or_else(|| CmdError::msg("Usage: +humanity/set <target>=<0..10>"))?;
    let target = match_object(ctx.w, ctx.actor, target_spec)?;
    let mut doc = cmd
        .read(ctx.w, target)
        .ok_or_else(|| CmdError::msg("That character has no sheet."))?;
    if doc.text("template") != Some("vampire") {
        return Err(CmdError::msg("That character is not Kindred."));
    }
    let store = cmd.store_for_doc(&doc);
    store
        .set_field(&mut doc, "humanity", serde_json::json!(n))
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_in(target, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!("Set {}'s morality to {n}.", name_of(ctx, target)),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

/// `+blood/heal [<n>]` — spend blood to mend bashing/lethal levels (1:1,
/// bashing first). Aggravated cannot be mended this way.
fn blood_heal(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    mut doc: SheetDoc,
    cur: i32,
    gt: GenTraits,
) -> Result<(), CmdError> {
    let want = arg_count(ctx, 1)?;
    if want < 1 {
        return Err(CmdError::msg("Heal how many levels?"));
    }
    let budget = want.min(cur).max(0) as u32;
    let mut track =
        crate::health::HealthTrack::from_marks(&doc.track("health").unwrap_or_default());
    let mut healed = track.heal(crate::health::Damage::Bashing, budget);
    healed += track.heal(crate::health::Damage::Lethal, budget - healed);
    if healed == 0 {
        return Err(CmdError::msg(
            "You have no bashing or lethal damage to mend (aggravated needs more than blood).",
        ));
    }
    doc.set_track("health", &track.to_marks());
    let store = cmd.store_for_doc(&doc);
    store
        .set_field(
            &mut doc,
            "bloodpool",
            serde_json::json!(cur - healed as i32),
        )
        .map_err(|e| CmdError::msg(e.to_string()))?;
    store
        .put_in(ctx.actor, &doc, ctx.actor, ctx)
        .map_err(|es| CmdError::msg(fmt_errs(&es)))?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "wod",
        format!(
            "You channel {healed} blood, mending {healed} health level(s). Blood {}/{}.",
            cur - healed as i32,
            gt.max_blood
        ),
    );
    Ok(())
}

/// Validate + persist a new Blood Pool value.
fn set_blood(
    cmd: &WodCmd,
    ctx: &mut Ctx<'_>,
    doc: &mut SheetDoc,
    value: i32,
) -> Result<(), CmdError> {
    let store = cmd.store_for_doc(doc);
    store
        .set_field(doc, "bloodpool", serde_json::json!(value.max(0)))
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

/// Register the Kindred command set.
pub(crate) fn register_commands(reg: &mut Registrar<'_>, cmd: Arc<WodCmd>) -> Result<(), ModError> {
    let disc_help = "+disc  --  List your Disciplines.\n\
        +disc/roll <discipline>+<attribute-or-ability> [vs <diff>]  --  Build and roll \
        the pool (e.g. +disc/roll celerity+dexterity vs 6). You are not Kindred? The \
        command refuses.";
    for name in ["+disc", "+disciplines"] {
        reg.command(
            CommandSpec {
                name: name.into(),
                switches: vec!["roll".into()],
                shape: ArgShape::Raw,
                help: disc_help.into(),
                ..CommandSpec::default()
            },
            {
                let cmd = Arc::clone(&cmd);
                Arc::new(move |ctx: &mut Ctx<'_>| disc_cmd(&cmd, ctx))
            },
        )?;
    }
    reg.command(
        CommandSpec {
            name: "+blood".into(),
            switches: vec!["spend".into(), "gain".into(), "heal".into(), "buff".into()],
            shape: ArgShape::Raw,
            help: "+blood  --  Show your Blood Pool.\n\
                   +blood/spend <n>  --  Spend blood (Discipline costs, waking, etc.).\n\
                   +blood/gain <n>  --  Feed (up to your generation's maximum).\n\
                   +blood/heal [<n>]  --  Spend n blood to heal n bashing/lethal levels.\n\
                   +blood/buff <n>  --  Flood a Physical Attribute (ST-adjudicated)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| blood_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+frenzy".into(),
            switches: vec!["fear".into()],
            shape: ArgShape::Raw,
            help: "+frenzy [vs <diff>]  --  Roll Self-Control/Instinct to resist frenzy.\n\
                   +frenzy/fear [vs <diff>]  --  Roll Courage to resist Rötschreck."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| frenzy_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+generation".into(),
            shape: ArgShape::Raw,
            help: "+generation  --  Show your generation and its Blood Pool / trait limits.".into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| generation_cmd(&cmd, ctx))
        },
    )?;
    reg.command(
        CommandSpec {
            name: "+humanity".into(),
            switches: vec!["set".into()],
            shape: ArgShape::EqPair,
            help: "+humanity  --  Show your Humanity / Path rating.\n\
                   +humanity/set <target>=<0..10>  --  Adjust a character's morality (staff)."
                .into(),
            ..CommandSpec::default()
        },
        {
            let cmd = Arc::clone(&cmd);
            Arc::new(move |ctx: &mut Ctx<'_>| humanity_cmd(&cmd, ctx))
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_table_matches_crib() {
        assert_eq!(generation_traits(13).max_blood, 10);
        assert_eq!(generation_traits(13).per_turn, 1);
        assert_eq!(generation_traits(9).per_turn, 2);
        assert_eq!(
            generation_traits(8),
            GenTraits {
                max_blood: 15,
                per_turn: 3,
                trait_max: 5
            }
        );
        assert_eq!(
            generation_traits(7),
            GenTraits {
                max_blood: 20,
                per_turn: 4,
                trait_max: 6
            }
        );
        assert_eq!(
            generation_traits(4),
            GenTraits {
                max_blood: 50,
                per_turn: 10,
                trait_max: 9
            }
        );
        // Thin-blooded / clamp.
        assert_eq!(generation_traits(15).max_blood, 10);
        assert_eq!(generation_traits(2).max_blood, 50); // clamped to 4th
    }

    #[test]
    fn clan_table_is_complete() {
        assert_eq!(CLANS.len(), 14);
        let brujah = clan_def("brujah").unwrap();
        assert_eq!(brujah.disciplines, &["Celerity", "Potence", "Presence"]);
        assert!(clan_def("caitiff").unwrap().disciplines.is_empty());
    }

    #[test]
    fn in_clan_discipline_xp_is_cheaper() {
        let d = crate::splats::Dispatch::build(
            "wod",
            crate::splats::all_splats(&["mortal", "vampire"]),
        );
        let mut doc = SheetDoc::new_default(&d.schemas.schema_for_id("vampire"));
        doc.set_text("template", "vampire");
        doc.set_text("clan", "brujah");
        // Celerity is in-clan for Brujah (×5); Auspex is not (×7). Raise 2→3.
        assert_eq!(discipline_raise_cost(&doc, "Celerity", 2, 3), 10); // 2×5
        assert_eq!(discipline_raise_cost(&doc, "Auspex", 2, 3), 14); // 2×7
        // A new Discipline (0→1) is a flat 10 either way.
        assert_eq!(discipline_raise_cost(&doc, "Auspex", 0, 1), 10);
        // Caitiff (no clan) pays ×6.
        doc.set_text("clan", "caitiff");
        assert_eq!(discipline_raise_cost(&doc, "Celerity", 2, 3), 12);
    }

    fn vampire_doc() -> SheetDoc {
        let d = crate::splats::Dispatch::build(
            "wod",
            crate::splats::all_splats(&["mortal", "vampire"]),
        );
        let mut doc = SheetDoc::new_default(&d.schemas.schema_for_id("vampire"));
        doc.set_text("template", "vampire");
        doc
    }

    #[test]
    fn finalize_derives_blood_humanity_and_weakness() {
        let mut doc = vampire_doc();
        doc.set_text("clan", "brujah");
        doc.set_number("conscience", 3);
        doc.set_number("self_control", 2);
        Vampire.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(
            doc.number("bloodpool"),
            Some(10),
            "13th-gen Blood Pool full"
        );
        assert_eq!(
            doc.number("humanity"),
            Some(5),
            "Conscience 3 + Self-Control 2"
        );
        assert!(
            doc.text("weakness")
                .unwrap()
                .to_lowercase()
                .contains("frenzy")
        );
    }

    #[test]
    fn path_swaps_the_morality_virtues() {
        let mut doc = vampire_doc();
        doc.set_text("path", "Path of Caine");
        doc.set_number("conviction", 3);
        doc.set_number("instinct", 4);
        // On a Path, morality derives from Conviction + Instinct.
        Vampire.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(doc.number("humanity"), Some(7));
        // And the chargen Virtue pool sources Conviction/Instinct/Courage.
        let c = Vampire.chargen_contribution(&WodConfig::default(), &doc);
        let virtues = c.pools.iter().find(|p| p.id == "virtues").unwrap();
        assert_eq!(
            virtues.source,
            SplatPoolSource::ScalarDotsAboveOne(vec![
                SmolStr::new("conviction"),
                SmolStr::new("instinct"),
                SmolStr::new("courage"),
            ])
        );
    }

    #[test]
    fn generation_is_chargen_locked() {
        // `generation` must be a locked field so `+stat generation=N` is
        // refused during chargen — otherwise a free 4th-gen (Blood Pool 50)
        // fledgling is a `+stat` away. It is derived from the Generation
        // background instead.
        assert!(Vampire.chargen_locked_fields().contains(&"generation"));
    }

    #[test]
    fn generation_background_lowers_generation() {
        let mut doc = vampire_doc();
        // Generation background of 3 dots → 10th generation (13 - 3).
        doc.map_set("backgrounds", "Generation", serde_json::json!(3));
        Vampire.finalize_starting(&WodConfig::default(), &mut doc);
        assert_eq!(doc.number("generation"), Some(10));
        assert_eq!(doc.number("bloodpool"), Some(13), "10th-gen Blood Pool max");
    }
}
