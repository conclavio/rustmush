//! **`@geology` — staff truth** (doc 20 cut SM.1; §3.2, D-20-4; the SM.1 brief's lead ruling 2).
//!
//! `@geology <body> <x>,<y> [province|field|pocket]` prints the keyed ground at one point of a
//! body's pocket lattice: its province's survey words, its field's words and ore-pocket counts,
//! and the pocket itself, family by family. No site's authored shift applies. **Doc 20 cut
//! SM.4b** adds the site form SM.1 and SM.2 deferred: `@geology <site> <pocket>|<field>
//! [province|field|pocket]` — the site named as a player names it (`Ahuna Flank`, its location
//! or its entry, case-insensitive), the address in the site's own vocabulary
//! (`crate::surface::address`: `C117`, `C-NW`) — printing the same levels for that lattice
//! pocket **under the site's authored block shift**, exactly what the site's readings render
//! from. A pure read of the RAM [`GeologyState`](super::GeologyState): no document, no post, no
//! submission, registered in its own ReadOnly bracket. Staff only; it tells nobody else
//! anything.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CanonicalId, CmdError, CommandSpec, Ctx, World};

use super::geology::{FieldGround, OreFamily, PocketRow, ProvinceGround, Word, by_family};
use super::reading::READING_RULE_VERSION;
use super::{body_geology, geology, live};
use crate::commands::require_staff;

/// The usage line every malformed argument gets.
const USAGE: &str = "Usage: @geology <body> <x>,<y> [province|field|pocket]  or  @geology <site> <pocket>|<field> [province|field|pocket]";

/// Register `@geology` in its own ReadOnly bracket (the `@corridor` pattern), restoring the
/// module default after.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "@geology".into(),
            shape: ArgShape::Raw,
            help: "@geology <body> <x>,<y> [province|field|pocket]   (staff) the ground's truth at one point of a body's pocket lattice (40 m cells; x east, y north of the equator): its province's survey words, its field's words and ore-pocket counts, and the pocket itself, family by family. No site's authored shift applies.\n@geology <site> <pocket>|<field> [province|field|pocket]   (staff) the same at a mining site's address (C117, C-NW), under the site's authored block shift. It writes nothing and tells nobody."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(geology_cmd),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    Ok(())
}

/// Which lines to print.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    All,
    Province,
    Field,
    Pocket,
}

/// A site address (doc 20 cut SM.4b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SiteAddress {
    /// A pocket, site-local.
    Pocket((i64, i64)),
    /// A field, site-local.
    Field((i64, i64)),
}

/// What `@geology` was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Form {
    /// `<body> <x>,<y>`.
    Body {
        body: CanonicalId,
        point: (i64, i64),
    },
    /// `<site words…> <address>` (doc 20 cut SM.4b).
    Site {
        site: String,
        address: SiteAddress,
        text: String,
    },
}

/// A level word.
fn level_of(word: &str) -> Option<Level> {
    match word.to_ascii_lowercase().as_str() {
        "province" => Some(Level::Province),
        "field" => Some(Level::Field),
        "pocket" => Some(Level::Pocket),
        _ => None,
    }
}

/// **The grammar** (doc 20 cut SM.4b): an optional trailing level word; then the body form when
/// the last token carries a comma (`<body> <x>,<y>`, exactly two tokens), else the site form —
/// the last token a site address (`address::parse_pocket_address`, else
/// `address::parse_field_name`) and the words before it the site's name. `pocket` with a field
/// address is the usage. `None` for anything else.
fn parse(raw: &str) -> Option<(Form, Level)> {
    let mut words: Vec<&str> = raw.split_whitespace().collect();
    let level = match words.last().and_then(|word| level_of(word)) {
        Some(level) if words.len() >= 3 => {
            words.pop();
            level
        }
        _ => Level::All,
    };
    let last = *words.last()?;
    if last.contains(',') {
        let [body, point] = words.as_slice() else {
            return None;
        };
        let body = CanonicalId::new(*body).ok()?;
        let (x, y) = point.split_once(',')?;
        let point = (x.parse::<i64>().ok()?, y.parse::<i64>().ok()?);
        return Some((Form::Body { body, point }, level));
    }
    let site_words = words.get(..words.len().checked_sub(1)?)?;
    if site_words.is_empty() {
        return None;
    }
    let address = match super::address::parse_pocket_address(last) {
        Some(pocket) => SiteAddress::Pocket(pocket),
        None => SiteAddress::Field(super::address::parse_field_name(last)?),
    };
    if matches!(address, SiteAddress::Field(_)) && level == Level::Pocket {
        return None;
    }
    Some((
        Form::Site {
            site: site_words.join(" "),
            address,
            text: last.to_ascii_uppercase(),
        },
        level,
    ))
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

/// **The lines `@geology <site> <addr>` prints** (doc 20 cut SM.4b), built from the library's own
/// values under the site's authored shift: the header — `Geology of {site} {addr} — pocket
/// {gx},{gy} of {body} (key v{k}; reading rule v{r}; block {L} shift {s}):`, or `… — field
/// {fx},{fy} of {body} …` for a field address — then the province, field and (for a pocket) the
/// pocket lines of [`geology_lines`]. `field` is the site field's ground
/// (`chart::field_ground_at`); `row` the pocket's for a pocket address.
#[must_use]
#[allow(clippy::too_many_arguments)] // the header names each of them
pub fn site_geology_lines(
    site: &str,
    body: &CanonicalId,
    address: &str,
    lattice: (i64, i64),
    key_version: u32,
    block: (char, i8),
    field: &FieldGround,
    row: Option<&PocketRow>,
) -> Vec<String> {
    let what = match row {
        Some(_) => format!("pocket {},{}", lattice.0, lattice.1),
        None => format!("field {},{}", field.field.0, field.field.1),
    };
    let mut out = vec![format!(
        "Geology of {site} {address} — {what} of {body} (key v{key_version}; reading rule \
         v{READING_RULE_VERSION}; block {} shift {}):",
        block.0, block.1
    )];
    out.push(format!(
        "  province {},{}: {}.",
        field.province.province.0,
        field.province.province.1,
        province_words(&field.province)
    ));
    out.push(format!(
        "  field {},{}: {}.",
        field.field.0,
        field.field.1,
        field_words(field)
    ));
    if let Some(row) = row {
        out.push(format!(
            "  pocket {},{}: {}.",
            lattice.0,
            lattice.1,
            pocket_words(row)
        ));
    }
    out
}

fn geology_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_staff(ctx)?;
    let raw = ctx.args.raw().trim().to_owned();
    let (form, level) = parse(&raw).ok_or_else(|| CmdError::msg(USAGE))?;
    let (mac, key_version) = live(ctx.w).map_err(|disabled| {
        CmdError::msg(format!("Geology is disabled: {}.", disabled.reason()))
    })?;
    let out = match form {
        Form::Body { body, point } => body_lines(ctx.w, &body, point, &mac, key_version, level)?,
        Form::Site {
            site,
            address,
            text,
        } => site_lines(ctx.w, &site, address, &text, key_version, level)?,
    };
    let actor = ctx.actor;
    for line in out {
        ctx.out.sys_to_actor(actor, "tacsim", line);
    }
    Ok(())
}

/// The body form's lines (doc 20 cut SM.1).
fn body_lines(
    w: &World,
    body_id: &CanonicalId,
    point: (i64, i64),
    mac: &mush_core::GeologyMac,
    key_version: u32,
    level: Level,
) -> Result<Vec<String>, CmdError> {
    let body = body_geology(w, body_id)
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
    let (field, row) = geology::pocket_at(mac, &body, pocket).map_err(|_| off())?;
    let [header, province, field_line, pocket_line] =
        geology_lines(body_id, pocket, key_version, &field, &row);
    let mut out = vec![header];
    match level {
        Level::All => out.extend([province, field_line, pocket_line]),
        Level::Province => out.push(province),
        Level::Field => out.push(field_line),
        Level::Pocket => out.push(pocket_line),
    }
    Ok(out)
}

/// **The site form's lines** (doc 20 cut SM.4b): the site resolved by its name
/// (`chart::site_named`), its keyed ground (`chart::site_context`), the field's ground under its
/// block's authored shift (`chart::field_ground_at`) and, for a pocket, the pocket's row
/// (`chart::pocket_at`) — the very functions the site's readings render through.
fn site_lines(
    w: &World,
    name: &str,
    address: SiteAddress,
    text: &str,
    key_version: u32,
    level: Level,
) -> Result<Vec<String>, CmdError> {
    let site = super::chart::site_named(w, name)
        .ok_or_else(|| CmdError::msg(format!("There is no mining site called {name}.")))?;
    let ctx = super::chart::site_context(w, &site.entry)
        .map_err(|dark| CmdError::msg(format!("Geology is disabled: {}.", dark.reason())))?;
    let unreadable = || CmdError::msg(format!("{text} does not read on {}.", site.name));
    let (field, pocket) = match address {
        SiteAddress::Pocket(pocket) => (
            super::address::field_of_pocket(pocket).ok_or_else(unreadable)?,
            Some(pocket),
        ),
        SiteAddress::Field(field) => (field, None),
    };
    let ground = super::chart::field_ground_at(&ctx, field).map_err(|_| unreadable())?;
    let row = match pocket {
        Some(pocket) => Some(super::chart::pocket_at(&ctx, pocket).map_err(|_| unreadable())?),
        None => None,
    };
    let block_index = usize::try_from(
        field
            .1
            .div_euclid(2)
            .saturating_mul(super::ground::SITE_BLOCKS)
            .saturating_add(field.0.div_euclid(2)),
    )
    .unwrap_or(0);
    let shift = ctx.spec.shifts.get(block_index).copied().unwrap_or(0);
    let letter = super::address::letter_of_block(field.0.div_euclid(2), field.1.div_euclid(2))
        .unwrap_or('?');
    let lattice = pocket.map_or((0, 0), |(px, py)| {
        let raw = (
            ctx.spec.origin.0.saturating_add(px),
            ctx.spec.origin.1.saturating_add(py),
        );
        geology::normalise_pocket(&ctx.body, raw).unwrap_or(raw)
    });
    let lines = site_geology_lines(
        &site.name,
        &ctx.spec.body,
        text,
        lattice,
        key_version,
        (letter, shift),
        &ground,
        row.as_ref(),
    );
    let mut lines = lines.into_iter();
    let header = lines.next().unwrap_or_default();
    let province = lines.next().unwrap_or_default();
    let field_line = lines.next().unwrap_or_default();
    let pocket_line = lines.next();
    let mut out = vec![header];
    match level {
        Level::All => {
            out.push(province);
            out.push(field_line);
            out.extend(pocket_line);
        }
        Level::Province => out.push(province),
        Level::Field => out.push(field_line),
        Level::Pocket => out.extend(pocket_line),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The grammar** (doc 20 cut SM.4b): the body form by its comma, the site form with a
    /// multi-word name and either address, the level word last; malformed is `None`.
    #[test]
    fn sm4b_geology_parses_both_forms() {
        let body = parse("ceres 1200,-340").expect("the body form");
        assert_eq!(
            body,
            (
                Form::Body {
                    body: CanonicalId::new("ceres").expect("an id"),
                    point: (1_200, -340)
                },
                Level::All
            )
        );
        assert_eq!(
            parse("ceres 1200,-340 field").map(|p| p.1),
            Some(Level::Field)
        );
        let site = parse("Ahuna Flank c117").expect("the site form");
        assert_eq!(
            site,
            (
                Form::Site {
                    site: "Ahuna Flank".to_owned(),
                    address: SiteAddress::Pocket((36, 56)),
                    text: "C117".to_owned(),
                },
                Level::All
            )
        );
        assert_eq!(
            parse("ahuna_port C-NW province"),
            Some((
                Form::Site {
                    site: "ahuna_port".to_owned(),
                    address: SiteAddress::Field((4, 7)),
                    text: "C-NW".to_owned(),
                },
                Level::Province
            ))
        );
        for bad in [
            "",
            "C117",
            "ahuna C-NW pocket",
            "ahuna Q1",
            "ceres 1,2,3",
            "ceres one,two",
            "ceres 1,2 lots",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }
}
