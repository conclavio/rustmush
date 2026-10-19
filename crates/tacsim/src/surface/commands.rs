//! **`@geology` — staff truth** (doc 20 cut SM.1; §3.2, D-20-4; the SM.1 brief's lead ruling 2).
//!
//! `@geology <body> <x>,<y> [province|field|pocket]` prints the keyed ground at one point of a
//! body's pocket lattice: its province's survey words, its field's words and ore-pocket counts,
//! and the pocket itself, family by family. No site's authored shift applies (sites are SM.2's,
//! and so is the `@geology <site> <addr>` form doc 20 §3.2 names). A pure read of the RAM
//! [`GeologyState`](super::GeologyState): no document, no post, no submission, registered in its
//! own ReadOnly bracket. Staff only; it tells nobody else anything.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CanonicalId, CmdError, CommandSpec, Ctx};

use super::geology::{FieldGround, OreFamily, PocketRow, ProvinceGround, Word, by_family};
use super::reading::READING_RULE_VERSION;
use super::{body_geology, geology, live};
use crate::commands::require_staff;

/// The usage line every malformed argument gets.
const USAGE: &str = "Usage: @geology <body> <x>,<y> [province|field|pocket]";

/// Register `@geology` in its own ReadOnly bracket (the `@corridor` pattern), restoring the
/// module default after.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "@geology".into(),
            shape: ArgShape::Raw,
            help: "@geology <body> <x>,<y> [province|field|pocket]   (staff) the ground's truth at one point of a body's pocket lattice (40 m cells; x east, y north of the equator): its province's survey words, its field's words and ore-pocket counts, and the pocket itself, family by family. No site's authored shift applies. It writes nothing and tells nobody."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(geology_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    Ok(())
}

/// Which lines to print.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    All,
    Province,
    Field,
    Pocket,
}

/// `<body> <x>,<y> [level]`, or `None`.
fn parse(raw: &str) -> Option<(CanonicalId, (i64, i64), Level)> {
    let mut words = raw.split_whitespace();
    let body = CanonicalId::new(words.next()?).ok()?;
    let (x, y) = words.next()?.split_once(',')?;
    let point = (x.parse::<i64>().ok()?, y.parse::<i64>().ok()?);
    let level = match words.next() {
        None => Level::All,
        Some(word) => match word.to_ascii_lowercase().as_str() {
            "province" => Level::Province,
            "field" => Level::Field,
            "pocket" => Level::Pocket,
            _ => return None,
        },
    };
    if words.next().is_some() {
        return None;
    }
    Some((body, point, level))
}

/// `metals lean, volatiles rich, …`.
fn province_words(province: &ProvinceGround) -> String {
    OreFamily::ALL
        .map(|family| {
            format!(
                "{} {}",
                family.label(),
                by_family(province.words, family).name()
            )
        })
        .join(", ")
}

/// `metals lean (2 pockets), …`.
fn field_words(field: &FieldGround) -> String {
    OreFamily::ALL
        .map(|family| {
            let count = by_family(field.counts, family);
            let noun = if count == 1 { "pocket" } else { "pockets" };
            format!(
                "{} {} ({count} {noun})",
                family.label(),
                by_family(field.words, family).name()
            )
        })
        .join(", ")
}

/// `metals lean 4 t; volatiles fair 9 t`, or `barren`.
fn pocket_words(pocket: &PocketRow) -> String {
    let ore: Vec<String> = OreFamily::ALL
        .into_iter()
        .filter_map(|family| {
            let class = by_family(pocket.classes, family);
            (class != Word::Barren).then(|| {
                format!(
                    "{} {} {} t",
                    family.label(),
                    class.name(),
                    by_family(pocket.tonnes, family)
                )
            })
        })
        .collect();
    if ore.is_empty() {
        "barren".to_owned()
    } else {
        ore.join("; ")
    }
}

/// The lines `@geology` prints for one point (the header first), built from the library's own
/// values — what the test compares against.
#[must_use]
pub fn geology_lines(
    body: &CanonicalId,
    pocket: (i64, i64),
    key_version: u32,
    field: &FieldGround,
    row: &PocketRow,
) -> [String; 4] {
    [
        format!(
            "Geology of {body} at pocket {},{} (key v{key_version}; reading rule \
             v{READING_RULE_VERSION}):",
            pocket.0, pocket.1
        ),
        format!(
            "  province {},{}: {}.",
            field.province.province.0,
            field.province.province.1,
            province_words(&field.province)
        ),
        format!(
            "  field {},{}: {}.",
            field.field.0,
            field.field.1,
            field_words(field)
        ),
        format!("  pocket {},{}: {}.", pocket.0, pocket.1, pocket_words(row)),
    ]
}

fn geology_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_staff(ctx)?;
    let raw = ctx.args.raw().trim().to_owned();
    let (body_id, point, level) = parse(&raw).ok_or_else(|| CmdError::msg(USAGE))?;
    let (mac, key_version) = live(ctx.w).map_err(|disabled| {
        CmdError::msg(format!("Geology is disabled: {}.", disabled.reason()))
    })?;
    let body = body_geology(ctx.w, &body_id)
        .ok_or_else(|| CmdError::msg(format!("No geology table is loaded for {body_id}.")))?;
    let lattice = body.lattice();
    let off = || {
        CmdError::msg(format!(
            "That point is off {body_id}: y runs from {} to {}.",
            lattice.south(),
            lattice.north().saturating_sub(1)
        ))
    };
    let pocket = geology::normalise_pocket(&body, point).map_err(|_| off())?;
    let (field, row) = geology::pocket_at(&mac, &body, pocket).map_err(|_| off())?;
    let [header, province, field_line, pocket_line] =
        geology_lines(&body_id, pocket, key_version, &field, &row);
    let mut out = vec![header];
    match level {
        Level::All => out.extend([province, field_line, pocket_line]),
        Level::Province => out.push(province),
        Level::Field => out.push(field_line),
        Level::Pocket => out.push(pocket_line),
    }
    let actor = ctx.actor;
    for line in out {
        ctx.out.sys_to_actor(actor, "tacsim", line);
    }
    Ok(())
}
