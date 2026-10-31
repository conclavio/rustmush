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
//! Nothing is stored and nothing in the world changes: a check is a read. The
//! in-instance registry (publishing a design) is cut UD.3's.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use mush_core::module::InvokeHandler;
use mush_core::output::ConnId;
use mush_core::{Ctx, ModError, Registrar};
use mush_proto::{InvokeRequest, InvokeResult, UnitDesignRequest, UnitDesignResponse};
use serde_json::{Value, json};

use crate::config::LoadedPack;
use crate::definitions::Catalog;

/// The invoke service's name.
pub const SERVICE: &str = "unit_design";

/// The prefix every designed unit's id carries.
pub const UNIT_PREFIX: &str = "tacsim.unit.";

/// The prefix of the pack a design ships in (`tacsim.pack.<slug>`).
pub const PACK_PREFIX: &str = "tacsim.pack.";

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
    let loaded = match crate::pack::load_pack(&text) {
        Ok(loaded) => loaded,
        Err(error) => return DesignCheck::refused(error.to_string()),
    };
    let packs = [LoadedPack {
        source: format!("<design {id}>"),
        pack: loaded,
    }];
    match Catalog::build_fielded(&packs, crate::NS) {
        Ok(_) => DesignCheck {
            refusals: Vec::new(),
            pack: Some(pack),
        },
        Err(error) => DesignCheck::refused(error.to_string()),
    }
}

/// The `unit_design` invoke handler: staff only.
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
        let UnitDesignRequest::Check { id, version, rules } = request;
        let found = check(id, *version, rules);
        InvokeResult::UnitDesign(UnitDesignResponse::Checked {
            refusals: found.refusals,
            pack: found.pack,
        })
    }
}

/// Register the `unit_design` invoke handler.
///
/// # Errors
///
/// The registrar's, for a second registration.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.invoke_handler(SERVICE, Arc::new(DesignService))
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
