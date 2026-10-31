//! **The staff unit designer's server half** (doc 31 cut UD.2; D-31-7 as
//! amended on 2026-10-30: the rules are checked here, by the loader and the
//! catalogue build the game itself runs, never by a second implementation in
//! the browser).
//!
//! The web studio sends a design — a unit id, a version and the unit's `def`
//! object — as a `unit_design` invoke. [`check`] makes the one-entry rules pack
//! the design would ship as (`crates/tacsim/content/units/<slug>.json`): its
//! `requires`, the packs that carry the weapons, sensors and cores the `def`
//! names, found in the shipped content; and its `digest`, the content layer's
//! own. It then loads that pack through [`crate::pack::load_pack`] and builds
//! the fielded catalogue with it ([`Catalog::build_fielded`]): every bound,
//! every cross-check and the binding to a tactical unit, exactly as a boot
//! would. A refusal is the loader's or the build's own sentence; a design that
//! holds comes back as its finished pack, ready to commit.
//!
//! A check stores nothing. **Publishing** (cut UD.3, D-31-9) checks the same
//! way and then keeps the design as a registry row, `tacsim:design:{id}@{version}`
//! in this module's namespace: its finished pack and its look. The host reads
//! those rows before this module registers ([`design_sources`]) and hands the
//! packs in beside `[tacsim].pack_paths`, so a published design is fieldable
//! after the next restart (Q-31-1) and the boot's own pack ledger records it as
//! it records any operator pack. A version, once published, never changes; a
//! design is withdrawn ([`DesignRecord::withdrawn`]), never deleted, because a
//! hull or a match record may name it.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use mush_core::module::InvokeHandler;
use mush_core::output::ConnId;
use mush_core::{Ctx, ModError, Registrar};
use mush_proto::{InvokeRequest, InvokeResult, UnitDesignRequest, UnitDesignResponse};
use serde_json::{Value, json};

use crate::config::{LoadedPack, PackSource};
use crate::definitions::Catalog;

/// The invoke service's name.
pub const SERVICE: &str = "unit_design";

/// The prefix every designed unit's id carries.
pub const UNIT_PREFIX: &str = "tacsim.unit.";

/// The prefix a **published** design's id carries (D-31-9: its own
/// namespace, so a published design never shadows a unit shipped later).
pub const DESIGN_UNIT_PREFIX: &str = "tacsim.unit.design.";

/// The prefix of the pack a design ships in (`tacsim.pack.<slug>`).
pub const PACK_PREFIX: &str = "tacsim.pack.";

/// **The pulled feed a client asks for the looks of published designs on**
/// (doc 31 cut UD.3b, D-31-16): `{"types": [<unit type>, …]}` (at most
/// [`MAX_LOOK_TYPES`]) is answered `{"looks": {<type>: <look>}}` for each
/// type that is a published design (its newest version's look); any other
/// type is left out. A look is how a unit is drawn — label, doll, model — and
/// never what a match holds, so a client may ask for any type it has seen.
pub const LOOKS_FEED: &str = "unit.looks";

/// The most types one `unit.looks` query names.
pub const MAX_LOOK_TYPES: usize = 8;

/// The prefix of a design registry row's key.
pub const DESIGN_PREFIX: &str = "tacsim:design:";

/// A design registry row's key, `tacsim:design:{id}@{version}`.
#[must_use]
pub fn design_key(id: &str, version: u32) -> smol_str::SmolStr {
    smol_str::SmolStr::new(format!("{DESIGN_PREFIX}{id}@{version}"))
}

/// **One published design** (doc 31 cut UD.3): its id and version, the
/// finished pack the server built when it was published, its look, and
/// whether it is withdrawn.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignRecord {
    /// The unit's definition id, `tacsim.unit.<slug>`.
    pub id: String,
    /// The pack's version.
    pub version: u32,
    /// The finished rules pack (`requires` and `digest` filled in).
    pub pack: Value,
    /// The look, as the designer sent it.
    pub look: Value,
    /// Withdrawn: still loaded (a hull built on it resolves), never offered
    /// to a new match.
    #[serde(default)]
    pub withdrawn: bool,
}

/// **The packs of the published designs**, from this module's documents
/// `(key, value)` — what the host hands in beside `[tacsim].pack_paths` before
/// the module registers. A row that will not read, or whose key is not its
/// own, is skipped loudly; withdrawn designs are included (a hull may name
/// one). In key order, so a boot is deterministic.
#[must_use]
pub fn design_sources(rows: &[(String, Value)]) -> Vec<PackSource> {
    let mut rows: Vec<&(String, Value)> = rows
        .iter()
        .filter(|(key, _)| key.starts_with(DESIGN_PREFIX))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    for (key, value) in rows {
        match serde_json::from_value::<DesignRecord>(value.clone()) {
            Ok(record) if design_key(&record.id, record.version) == key.as_str() => {
                match serde_json::to_string_pretty(&record.pack) {
                    Ok(bytes) => out.push(PackSource {
                        name: format!("<design {}@{}>", record.id, record.version),
                        bytes,
                    }),
                    Err(error) => {
                        tracing::warn!(%key, %error, "tacsim: a design's pack will not encode; skipped");
                    }
                }
            }
            _ => tracing::warn!(%key, "tacsim: a design row is unreadable; skipped"),
        }
    }
    out
}

/// What a check found: the refusals (empty when the design holds) and, when it
/// holds, the finished pack.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignCheck {
    /// Each reason the design was refused, in the order found.
    pub refusals: Vec<String>,
    /// The finished pack, when there is no refusal.
    pub pack: Option<Value>,
}

impl DesignCheck {
    fn refused(why: impl Into<String>) -> Self {
        Self {
            refusals: vec![why.into()],
            pack: None,
        }
    }
}

/// Every entry the shipped, fielded content defines, with the pack (id and
/// version) it is in and the entry's kind. Read once.
fn shipped_entries() -> &'static BTreeMap<String, (String, u64, String)> {
    static ENTRIES: OnceLock<BTreeMap<String, (String, u64, String)>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let mut out = BTreeMap::new();
        for source in crate::definitions::content::fielded_sources() {
            let Ok(pack) = serde_json::from_str::<Value>(&source.bytes) else {
                continue;
            };
            let manifest = &pack["manifest"];
            let (Some(id), Some(version)) = (manifest["id"].as_str(), manifest["version"].as_u64())
            else {
                continue;
            };
            for (entry, body) in pack["entries"].as_object().into_iter().flatten() {
                let kind = body["kind"].as_str().unwrap_or_default().to_string();
                out.insert(entry.clone(), (id.to_string(), version, kind));
            }
        }
        out
    })
}

/// Every string anywhere in `v`.
fn strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Array(list) => list.iter().for_each(|v| strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

/// **The packs a unit `def` requires**: each shipped pack holding an entry the
/// `def` names by id (its weapons, sensors and cores), in the order first
/// named, once each.
#[must_use]
pub fn requires_of(rules: &Value) -> Vec<(String, u64)> {
    let mut named = Vec::new();
    strings(rules, &mut named);
    let mut out: Vec<(String, u64)> = Vec::new();
    for s in named {
        if let Some((pack, version, kind)) = shipped_entries().get(s)
            && kind != "unit"
            && !out.iter().any(|(p, _)| p == pack)
        {
            out.push((pack.clone(), *version));
        }
    }
    out
}

/// **Check one design** (see the module docs): its id, the pack it makes, the
/// loader and the fielded catalogue build. Never panics; every failure is a
/// refusal.
#[must_use]
pub fn check(id: &str, version: u32, rules: &Value) -> DesignCheck {
    let Some(slug) = id.strip_prefix(UNIT_PREFIX).filter(|s| !s.is_empty()) else {
        return DesignCheck::refused(format!("a designed unit's id is `{UNIT_PREFIX}<name>`"));
    };
    if let Err(error) = mush_core::CanonicalId::new(id) {
        return DesignCheck::refused(format!("`{id}` is not a canonical id: {error}"));
    }
    if shipped_entries().contains_key(id) {
        return DesignCheck::refused(format!("`{id}` is a shipped unit; a design takes a new id"));
    }
    let pack_id = format!("{PACK_PREFIX}{slug}");
    if shipped_entries()
        .values()
        .any(|(pack, _, _)| *pack == pack_id)
    {
        return DesignCheck::refused(format!(
            "`{pack_id}` is a shipped pack; a design takes a new id"
        ));
    }
    let requires: Vec<Value> = requires_of(rules)
        .into_iter()
        .map(|(id, version)| json!({ "id": id, "version": version }))
        .collect();
    let mut pack = json!({
        "manifest": {
            "id": pack_id,
            "version": version,
            "kind": "rules_definition",
            "digest": "0".repeat(64),
            "requires": requires,
        },
        "entries": { id: { "kind": "unit", "def": rules } },
    });
    // The content layer's own digest over the pack as it stands.
    let digest = match serde_json::to_string(&pack)
        .map_err(|e| e.to_string())
        .and_then(|text| mush_content::parse_pack(&text).map_err(|e| e.to_string()))
        .and_then(|content| crate::pack::rules_pack_of(content).map_err(|e| e.to_string()))
        .and_then(|rules| rules.computed_digest().map_err(|e| e.to_string()))
    {
        Ok(digest) => digest,
        Err(why) => return DesignCheck::refused(why),
    };
    pack["manifest"]["digest"] = Value::String(digest.to_hex());
    let text = match serde_json::to_string_pretty(&pack) {
        Ok(text) => text,
        Err(error) => return DesignCheck::refused(error.to_string()),
    };
    // The host's own verification, as a boot runs it over the design as an
    // operator pack: the loader, the digest and the `requires` (against the
    // shipped content), so a check never passes a pack a boot would refuse.
    let cfg = crate::TacsimConfig {
        enabled: true,
        packs: vec![PackSource {
            name: format!("<design {id}>"),
            bytes: text,
        }],
        ..crate::TacsimConfig::default()
    };
    let packs: Vec<LoadedPack> = match crate::verify_sources(&cfg) {
        Ok(packs) => packs,
        Err(error) => return DesignCheck::refused(error),
    };
    match Catalog::build_fielded(&packs, crate::NS) {
        Ok(_) => DesignCheck {
            refusals: Vec::new(),
            pack: Some(pack),
        },
        Err(error) => DesignCheck::refused(error.to_string()),
    }
}

/// The `unit_design` invoke handler: staff only. A check reads; a publish
/// writes one registry row.
struct DesignService;

impl InvokeHandler for DesignService {
    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        _conn: ConnId,
        _id: u64,
        request: &InvokeRequest,
    ) -> InvokeResult {
        let error = |code: &str, message: &str| {
            InvokeResult::UnitDesign(UnitDesignResponse::Error {
                code: code.into(),
                message: message.into(),
            })
        };
        let InvokeRequest::UnitDesign(request) = request else {
            return error("invalid", "Not a unit design request.");
        };
        if request.validate().is_err() {
            return error("invalid", "Invalid or oversized structured request.");
        }
        if !crate::commands::is_staff(ctx.w, ctx.actor) {
            return error("staff", "The unit designer is for staff.");
        }
        match request {
            UnitDesignRequest::Check { id, version, rules } => {
                let found = check(id, *version, rules);
                InvokeResult::UnitDesign(UnitDesignResponse::Checked {
                    refusals: found.refusals,
                    pack: found.pack,
                })
            }
            UnitDesignRequest::Publish {
                id,
                version,
                rules,
                look,
            } => {
                let refusals = publish(ctx, id, *version, rules, look);
                InvokeResult::UnitDesign(UnitDesignResponse::Published {
                    id: id.clone(),
                    version: *version,
                    refusals,
                })
            }
        }
    }
}

/// **Publish a design** (doc 31 cut UD.3, D-31-9): refused when the version
/// is already published — as a design, or as a pack this world's ledger
/// carries under the same id and version — or when the check refuses it;
/// else its registry row is written. The refusals, empty when it was
/// published.
fn publish(ctx: &mut Ctx<'_>, id: &str, version: u32, rules: &Value, look: &Value) -> Vec<String> {
    if !id.starts_with(DESIGN_UNIT_PREFIX) || id.len() == DESIGN_UNIT_PREFIX.len() {
        return vec![format!(
            "a published design's id is `{DESIGN_UNIT_PREFIX}<name>`, so it never shadows a shipped unit"
        )];
    }
    let Some(store) = ctx.w.module_state::<crate::TacsimStore>() else {
        return vec!["The TacSim layer is not enabled here.".into()];
    };
    if store.designs.contains_key(&(id.to_string(), version)) {
        return vec![format!(
            "{id}@{version} is already published; a published version never changes (publish {})",
            version + 1
        )];
    }
    if let Some(slug) = id.strip_prefix(UNIT_PREFIX) {
        let pack_id = format!("{PACK_PREFIX}{slug}");
        if store
            .published_keys
            .iter()
            .any(|(p, v)| p.as_str() == pack_id && *v == version)
        {
            return vec![format!(
                "{pack_id}@{version} is already on this world's pack ledger"
            )];
        }
    }
    let found = check(id, version, rules);
    let Some(pack) = found.pack else {
        return found.refusals;
    };
    crate::store::put_design(
        ctx,
        DesignRecord {
            id: id.to_string(),
            version,
            pack,
            look: look.clone(),
            withdrawn: false,
        },
    );
    Vec::new()
}

/// `@design`'s help.
const DESIGN_HELP: &str = "@design/list                        (staff) every published unit design: \
its id, version, and whether it is withdrawn.\n\
@design/show <id>[@<version>]       (staff) one design: its pack, digest, requires and model.\n\
@design/withdraw <id>[@<version>]   (staff) withdraw a design (every version, or one): it stays \
loaded, so a hull built on it still resolves, but is no longer offered to a new match. Designs \
are published from the web unit designer (doc 31); one becomes fieldable after the next restart.";

/// `@design` — the registry from the line client (doc 31 cut UD.3).
fn design_cmd(ctx: &mut Ctx<'_>) -> Result<(), mush_core::CmdError> {
    crate::commands::require_staff(ctx)?;
    let say = |ctx: &mut Ctx<'_>, line: String| {
        let actor = ctx.actor;
        ctx.out.sys_to_actor(actor, "tacsim", line);
    };
    let Some(store) = ctx.w.module_state::<crate::TacsimStore>() else {
        return Err(mush_core::CmdError::msg(
            "The TacSim layer is not enabled here.",
        ));
    };
    let word = ctx.args.raw().trim().to_string();
    let (want_id, want_version) = match word.split_once('@') {
        Some((id, v)) => (id.to_string(), v.parse::<u32>().ok()),
        None => (word.clone(), None),
    };
    let matches: Vec<DesignRecord> = store
        .designs()
        .filter(|d| d.id == want_id && want_version.is_none_or(|v| d.version == v))
        .cloned()
        .collect();
    if ctx.switches.has("withdraw") {
        if matches.is_empty() {
            return Err(mush_core::CmdError::msg(format!("No such design: {word}.")));
        }
        for mut record in matches {
            if !record.withdrawn {
                record.withdrawn = true;
                let line = format!("Withdrawn: {}@{}.", record.id, record.version);
                crate::store::put_design(ctx, record);
                say(ctx, line);
            }
        }
        return Ok(());
    }
    if ctx.switches.has("show") {
        if matches.is_empty() {
            return Err(mush_core::CmdError::msg(format!("No such design: {word}.")));
        }
        for record in matches {
            let manifest = &record.pack["manifest"];
            let requires: Vec<String> = manifest["requires"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| Some(format!("{}@{}", r["id"].as_str()?, r["version"])))
                .collect();
            let line = format!(
                "{}@{}{}  pack {}  digest {}  requires {}  model {}",
                record.id,
                record.version,
                if record.withdrawn { " (withdrawn)" } else { "" },
                manifest["id"].as_str().unwrap_or("?"),
                manifest["digest"].as_str().unwrap_or("?"),
                if requires.is_empty() {
                    "nothing".to_string()
                } else {
                    requires.join(", ")
                },
                record.look["model"].as_str().unwrap_or("none"),
            );
            say(ctx, line);
        }
        return Ok(());
    }
    let rows: Vec<String> = store
        .designs()
        .map(|d| {
            format!(
                "{}@{}{}",
                d.id,
                d.version,
                if d.withdrawn { "  withdrawn" } else { "" }
            )
        })
        .collect();
    if rows.is_empty() {
        say(ctx, "No unit design is published.".into());
    }
    for row in rows {
        say(ctx, row);
    }
    Ok(())
}

/// `unit.looks`'s handler (see [`LOOKS_FEED`]).
struct LooksQuery;

/// The `unit.looks` keyframe for `params`, over `store`'s designs.
#[must_use]
pub fn looks_frame(store: Option<&crate::TacsimStore>, params: &Value) -> Value {
    let mut looks = serde_json::Map::new();
    let types = params
        .get("types")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(MAX_LOOK_TYPES);
    if let Some(store) = store {
        for unit in types {
            let newest = store
                .designs()
                .filter(|d| d.id == unit)
                .max_by_key(|d| d.version);
            if let Some(design) = newest {
                looks.insert(unit.to_string(), design.look.clone());
            }
        }
    }
    json!({
        "schema": { "id": LOOKS_FEED, "version": 1 },
        "looks": looks,
    })
}

impl mush_core::QueryHandler for LooksQuery {
    fn query(
        &self,
        w: &mush_core::World,
        _actor: mush_core::ObjId,
        params: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Ok(mush_proto::OutputEvent::Panel {
            feed: smol_str::SmolStr::new_static(LOOKS_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: looks_frame(w.module_state::<crate::TacsimStore>(), params),
        })
    }
}

/// Register the `unit_design` invoke handler.
///
/// # Errors
///
/// The registrar's, for a second registration.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.invoke_handler(SERVICE, Arc::new(DesignService))?;
    // Doc 31 cut UD.3b (D-31-16): the pulled looks feed. Registered as a feed
    // so it reaches the client manifest and a `PanelSubscribe` routes to its
    // query; nothing is ever pushed on it.
    drop(reg.panel_feed(LOOKS_FEED)?);
    reg.query_handler(LOOKS_FEED, Arc::new(LooksQuery))?;
    // `@design/withdraw` writes a registry row; the list and show only read.
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command(
        mush_core::CommandSpec {
            name: "@design".into(),
            shape: mush_core::ArgShape::Raw,
            switches: ["list", "show", "withdraw"]
                .iter()
                .copied()
                .map(smol_str::SmolStr::new)
                .collect(),
            help: DESIGN_HELP.into(),
            ..mush_core::CommandSpec::default()
        },
        Arc::new(design_cmd),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped laser tank's `def`, under a new id.
    fn laser_def() -> Value {
        let text = include_str!("../content/units/line_tank_laser.json");
        let pack: Value = serde_json::from_str(text).expect("JSON");
        pack["entries"]["tacsim.unit.line_tank_laser"]["def"].clone()
    }

    /// A shipped unit's rules under a new id hold, and come back as the pack
    /// the shipped file is — its `requires` the same, its digest the content
    /// layer's, loading through the loader as a file would.
    #[test]
    fn a_shipped_def_under_a_new_id_holds_with_its_requires() {
        let found = check("tacsim.unit.design_laser", 1, &laser_def());
        assert_eq!(found.refusals, Vec::<String>::new());
        let pack = found.pack.expect("a pack");
        let requires: Vec<&str> = pack["manifest"]["requires"]
            .as_array()
            .expect("requires")
            .iter()
            .filter_map(|r| r["id"].as_str())
            .collect();
        let shipped: Value =
            serde_json::from_str(include_str!("../content/units/line_tank_laser.json"))
                .expect("JSON");
        let mut want: Vec<&str> = shipped["manifest"]["requires"]
            .as_array()
            .expect("requires")
            .iter()
            .filter_map(|r| r["id"].as_str())
            .collect();
        let mut got = requires.clone();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want, "the same packs the shipped file requires");
        let text = serde_json::to_string_pretty(&pack).expect("text");
        let cfg = crate::TacsimConfig {
            enabled: true,
            packs: vec![PackSource {
                name: "design".into(),
                bytes: text.clone(),
            }],
            ..crate::TacsimConfig::default()
        };
        assert!(
            crate::verify_sources(&cfg).is_ok(),
            "it verifies as an operator pack, its requires resolved against the shipped content: {:?}",
            crate::verify_sources(&cfg).err()
        );
        assert!(
            crate::pack::load_pack(&text).is_ok(),
            "it loads as a file would"
        );
    }

    /// The loader's and the build's own sentences come back as refusals.
    #[test]
    fn what_does_not_hold_is_refused_with_the_servers_reason() {
        assert!(check("line_tank_x", 1, &laser_def()).refusals[0].contains(UNIT_PREFIX));
        assert!(
            check("tacsim.unit.line_tank", 1, &laser_def()).refusals[0].contains("shipped unit")
        );
        let mut no_sections = laser_def();
        no_sections["sections"] = json!([]);
        let found = check("tacsim.unit.design_empty", 1, &no_sections);
        assert!(found.pack.is_none());
        assert!(
            found.refusals[0].contains("section"),
            "{:?}",
            found.refusals
        );
        let mut unknown_weapon = laser_def();
        unknown_weapon["components"][0]["role"]["weapon"] = json!("tacsim.weapon.nonesuch");
        let found = check("tacsim.unit.design_nonesuch", 1, &unknown_weapon);
        assert!(found.pack.is_none(), "an unknown weapon does not resolve");
        assert!(!found.refusals.is_empty());
        let mut stray = laser_def();
        stray["colour"] = json!("red");
        assert!(check("tacsim.unit.design_stray", 1, &stray).pack.is_none());
    }
}
