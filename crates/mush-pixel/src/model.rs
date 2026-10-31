//! **The model catalogue** (doc 31 D-31-1..4, cut UD.1): the baked whole
//! assemblies a look may name, keyed by model id.
//!
//! A **model** is the art — one assembly from the owner's asset packs, baked
//! whole on the owner's machine by `scripts/unit-bake/bake.sh` (sprites are
//! whole-unit renders, so parts are never composited at draw, D-31-4); a
//! **look** is how a unit type is presented, and names its model
//! ([`crate::look::Look::model`]). Many looks may name one model.
//!
//! `models/` holds, for each model:
//!
//! - `<id>.<the first twelve hex digits of its SHA-256>.mupk` — its bake, a
//!   one-entry unit pack ([`crate::bake`], format 3). Immutable and
//!   content-named: a re-bake is a new name, so a client caches a file for
//!   good (D-31-2, D-31-11).
//! - `<id>.sheet.json` — its hand-drawn text sheet, the fallback a look on the
//!   model draws while the bake is in flight or where a picture has no chain
//!   (D-31-1; the `sprites` object of doc 22's look format, without
//!   `size_cm`, which is the look's).
//! - `index.json` — every model: its id, file, SHA-256, size in bytes, the
//!   baked `size_cm`, its sheet, and the asset pack and prefab it was baked
//!   from. The model the web fetches is the index's `file`.
//!
//! Native builds embed every model file (the build script lists `models/`,
//! D-31-3); the WebAssembly build embeds the index and the sheets only.

use std::sync::OnceLock;

use serde_json::Value;

use crate::look::{self, Sheet};

include!(concat!(env!("OUT_DIR"), "/models.rs"));

/// The catalogue's index, as shipped.
const INDEX_JSON: &str = include_str!("../models/index.json");

/// One model in the catalogue's index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelEntry {
    /// The model's id (`mc.st.tank_lvl2`): what a look names.
    pub id: String,
    /// Its bake's file under `models/`, content-named.
    pub file: String,
    /// The file's SHA-256, lower-case hex.
    pub sha256: String,
    /// The file's length, bytes.
    pub bytes: u64,
    /// The baked model's `(width, height)`, centimetres.
    pub size_cm: (u32, u32),
    /// Its fallback sheet's file under `models/`, if it has one.
    pub sheet: Option<String>,
    /// The asset pack it was baked from.
    pub asset_pack: String,
    /// The prefab the bake assembled.
    pub prefab: String,
}

/// Read the index: every model with its file, digest, size and provenance.
fn parse_index(text: &str) -> Result<Vec<ModelEntry>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let list = v
        .get("models")
        .and_then(Value::as_array)
        .ok_or("index: `models` must be a list")?;
    let mut out = Vec::new();
    for m in list {
        let s = |key: &str| -> Result<String, String> {
            m.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("index: a model's `{key}` must be a non-empty string"))
        };
        let pair = m
            .get("size_cm")
            .and_then(Value::as_array)
            .and_then(|a| match a.as_slice() {
                [w, h] => Some((
                    u32::try_from(w.as_u64()?).ok()?,
                    u32::try_from(h.as_u64()?).ok()?,
                )),
                _ => None,
            })
            .ok_or("index: `size_cm` must be [width, height]")?;
        out.push(ModelEntry {
            id: s("id")?,
            file: s("file")?,
            sha256: s("sha256")?,
            bytes: m
                .get("bytes")
                .and_then(Value::as_u64)
                .ok_or("index: `bytes` must be a count")?,
            size_cm: pair,
            sheet: m.get("sheet").and_then(Value::as_str).map(str::to_string),
            asset_pack: s("asset_pack")?,
            prefab: s("prefab")?,
        });
    }
    Ok(out)
}

/// **The catalogue**: every model the tree ships, sorted by id. An index that
/// does not parse is a build defect the catalogue test names; it reads as
/// empty here rather than taking a renderer down.
#[must_use]
pub fn index() -> &'static [ModelEntry] {
    static INDEX: OnceLock<Vec<ModelEntry>> = OnceLock::new();
    INDEX.get_or_init(|| parse_index(INDEX_JSON).unwrap_or_default())
}

/// The index entry for model `id`.
#[must_use]
pub fn entry(id: &str) -> Option<&'static ModelEntry> {
    index().iter().find(|m| m.id == id)
}

/// The model files this build embeds, by file name (none in the
/// WebAssembly build).
#[must_use]
pub fn embedded_files() -> &'static [(&'static str, &'static [u8])] {
    EMBEDDED_MODELS
}

/// The model sheets' file names and texts, for the tests.
#[must_use]
pub fn sheet_sources() -> &'static [(&'static str, &'static str)] {
    MODEL_SHEETS
}

/// **Model `id`'s fallback sheet**, parsed once.
#[must_use]
pub fn sheet(id: &str) -> Option<&'static Sheet> {
    static SHEETS: OnceLock<Vec<(String, Sheet)>> = OnceLock::new();
    let file = entry(id)?.sheet.as_deref()?;
    SHEETS
        .get_or_init(|| {
            MODEL_SHEETS
                .iter()
                .filter_map(|(name, text)| {
                    Some((name.to_string(), look::parse_sheet_text(text).ok()?))
                })
                .collect()
        })
        .iter()
        .find(|(name, _)| name == file)
        .map(|(_, sheet)| sheet)
}

/// **The model files a client still needs** to draw `unit_types` baked
/// (doc 31 D-31-11): for each type, its look's model, when the catalogue
/// has it and it is not installed yet; `None` reads as an unclassified
/// contact, the `unknown` look. Each file once, in the order first needed.
#[must_use]
pub fn wanted_files<'a>(
    unit_types: impl IntoIterator<Item = Option<&'a str>>,
) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for unit in unit_types {
        let look = match unit {
            Some(u) => look::for_unit(u).or_else(look::unknown),
            None => look::unknown(),
        };
        let Some(id) = look.and_then(|l| l.model.as_deref()) else {
            continue;
        };
        if crate::bake::model(id).is_some() {
            continue;
        }
        if let Some(m) = entry(id)
            && !out.contains(&m.file.as_str())
        {
            out.push(m.file.as_str());
        }
    }
    out
}

/// Every `unit_type` a feed names, anywhere in it (a string a type, `null`
/// an unclassified contact), and every type in a `side_unit_types` list —
/// the cockpit frame's own side, sent so a client fetches those models
/// before the first contact (doc 31 D-31-11; the other side's types arrive
/// only with each classified contact): what [`wanted_files`] reads from the
/// feeds a client holds.
pub fn unit_types_in<'a>(feed: &'a Value, out: &mut Vec<Option<&'a str>>) {
    match feed {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "unit_type" {
                    match value {
                        Value::String(s) => out.push(Some(s.as_str())),
                        Value::Null => out.push(None),
                        _ => {}
                    }
                } else if key == "side_unit_types" {
                    out.extend(
                        value
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(Some),
                    );
                } else {
                    unit_types_in(value, out);
                }
            }
        }
        Value::Array(list) => list.iter().for_each(|v| unit_types_in(v, out)),
        _ => {}
    }
}

/// **The unit types the held feeds name that still lack a look** (doc 31 cut
/// UD.3b): [`unit_types_in`]'s types through [`crate::look::wanted`].
#[must_use]
pub fn wanted_looks<'a>(unit_types: impl IntoIterator<Item = Option<&'a str>>) -> Vec<&'a str> {
    crate::look::wanted(unit_types.into_iter().flatten())
}

/// Lower-case hex of `bytes`.
#[cfg(feature = "encode")]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The SHA-256 of `bytes`, lower-case hex.
#[cfg(feature = "encode")]
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex(&sha2::Sha256::digest(bytes))
}

/// The content-named file for model `id` whose bytes hash to `sha256`.
#[must_use]
pub fn file_name(id: &str, sha256: &str) -> String {
    format!("{id}.{}.mupk", sha256.get(..12).unwrap_or(sha256))
}

/// **Write one model into `dir`** (the reducer's and the catalogue's one
/// writer): `bake` as its own one-entry file, any older file of the same
/// model removed, and `index.json` rewritten with the model's entry (its
/// sheet named when `<id>.sheet.json` is beside it). Returns the entry.
///
/// # Errors
///
/// A description naming the file that could not be read or written.
#[cfg(feature = "encode")]
pub fn write_model(
    dir: &std::path::Path,
    bake: crate::bake::ModelBake,
) -> Result<ModelEntry, String> {
    let io = |path: &std::path::Path, e: std::io::Error| format!("{}: {e}", path.display());
    let id = bake.model.clone();
    let mut pack = crate::bake::Pack::default();
    let size_cm = bake.size_cm;
    let provenance = bake.provenance.clone();
    pack.put(bake);
    let bytes = pack.encode();
    let sha256 = sha256_hex(&bytes);
    let file = file_name(&id, &sha256);
    let prefix = format!("{id}.");
    for e in std::fs::read_dir(dir).map_err(|e| io(dir, e))?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix)
            && name.ends_with(".mupk")
            && name != file
            && name[prefix.len()..name.len() - 5].len() == 12
        {
            std::fs::remove_file(e.path()).map_err(|err| io(&e.path(), err))?;
        }
    }
    let path = dir.join(&file);
    std::fs::write(&path, &bytes).map_err(|e| io(&path, e))?;
    let sheet = format!("{id}.sheet.json");
    let entry = ModelEntry {
        id: id.clone(),
        file,
        sha256,
        bytes: bytes.len() as u64,
        size_cm,
        sheet: dir.join(&sheet).exists().then_some(sheet),
        asset_pack: provenance.asset_pack,
        prefab: provenance.prefab,
    };
    let index_path = dir.join("index.json");
    let mut models = match std::fs::read_to_string(&index_path) {
        Ok(text) => parse_index(&text).map_err(|e| format!("{}: {e}", index_path.display()))?,
        Err(_) => Vec::new(),
    };
    models.retain(|m| m.id != id);
    models.push(entry.clone());
    models.sort_by(|a, b| a.id.cmp(&b.id));
    std::fs::write(&index_path, index_text(&models)).map_err(|e| io(&index_path, e))?;
    Ok(entry)
}

/// The index's text for `models` (two-space JSON, a newline at the end).
#[cfg(feature = "encode")]
#[must_use]
pub fn index_text(models: &[ModelEntry]) -> String {
    let list: Vec<Value> = models
        .iter()
        .map(|m| {
            let mut o = serde_json::json!({
                "id": m.id,
                "file": m.file,
                "sha256": m.sha256,
                "bytes": m.bytes,
                "size_cm": [m.size_cm.0, m.size_cm.1],
                "asset_pack": m.asset_pack,
                "prefab": m.prefab,
            });
            if let (Some(sheet), Some(map)) = (&m.sheet, o.as_object_mut()) {
                map.insert("sheet".into(), sheet.as_str().into());
            }
            o
        })
        .collect();
    let mut text =
        serde_json::to_string_pretty(&serde_json::json!({ "models": list })).unwrap_or_default();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The catalogue holds** (doc 31 D-31-2, D-31-3; replaces U.6's pinned
    /// pack digest): the index parses; every model has exactly one embedded
    /// file, named by its own SHA-256, whose length and digest are the
    /// index's, which decodes to one entry of that model and holds the matrix
    /// check against the looks that name it, under rule 7's per-model
    /// ceiling; every sheet the index names is shipped and parses; and
    /// every shipped look's model is in the catalogue.
    #[test]
    #[cfg(feature = "encode")]
    fn the_catalogue_holds() {
        assert!(
            parse_index(INDEX_JSON).is_ok(),
            "{:?}",
            parse_index(INDEX_JSON)
        );
        let index = index();
        assert!(!index.is_empty());
        let mut ids: Vec<&str> = index.iter().map(|m| m.id.as_str()).collect();
        ids.dedup();
        assert_eq!(ids.len(), index.len(), "one entry a model");
        assert_eq!(
            embedded_files().len(),
            index.len(),
            "one file a model, no stale bake left in models/"
        );
        for m in index {
            let (_, bytes) = embedded_files()
                .iter()
                .find(|(f, _)| *f == m.file)
                .unwrap_or_else(|| panic!("{}: {} is not in models/", m.id, m.file));
            let sha = sha256_hex(bytes);
            assert_eq!(sha, m.sha256, "{}: the index's digest", m.file);
            assert_eq!(file_name(&m.id, &sha), m.file, "content-named");
            assert_eq!(bytes.len() as u64, m.bytes);
            assert!(
                bytes.len() <= crate::bake::BUDGET_BYTES,
                "{}: {} bytes, over rule 7's per-model {}",
                m.file,
                bytes.len(),
                crate::bake::BUDGET_BYTES
            );
            let pack = crate::bake::Pack::decode(bytes).expect("a model file decodes");
            assert_eq!(pack.entries.len(), 1, "{}: one model a file", m.file);
            let bake = &pack.entries[0];
            assert_eq!(bake.model, m.id);
            assert_eq!(bake.size_cm, m.size_cm);
            assert_eq!(
                crate::bake::check_model(bake, look::shipped()),
                Vec::<String>::new()
            );
            if let Some(sheet) = &m.sheet {
                assert!(
                    sheet_sources().iter().any(|(f, _)| f == sheet),
                    "{}: no {sheet}",
                    m.id
                );
                assert!(super::sheet(&m.id).is_some(), "{sheet} parses");
            }
        }
        for (file, text) in sheet_sources() {
            assert!(
                look::parse_sheet_text(text).is_ok(),
                "{file}: {:?}",
                look::parse_sheet_text(text)
            );
        }
        for l in look::shipped() {
            if let Some(id) = &l.model {
                assert!(
                    entry(id).is_some(),
                    "look {} names {id}, not in the catalogue",
                    l.id
                );
                assert!(crate::bake::model(id).is_some(), "{id}: embedded");
            }
        }
    }

    #[test]
    fn the_wanted_files_are_the_uninstalled_models_on_the_map() {
        // A feed naming the tank twice, the walker, and an unclassified contact.
        let feed = serde_json::json!({
            "unit_type": "tacsim.unit.line_tank",
            "side_unit_types": ["tacsim.unit.line_tank", "tacsim.unit.drone_mk1"],
            "contacts": [
                { "unit_type": "tacsim.unit.biped_mk1" },
                { "unit_type": null },
                { "unit_type": "tacsim.unit.line_tank_laser" },
            ],
        });
        let mut types = Vec::new();
        unit_types_in(&feed, &mut types);
        assert_eq!(types.len(), 6);
        let wanted = wanted_files(types);
        // Natively every model is embedded, so nothing is wanted; the
        // WebAssembly build wants each model once.
        if embedded_files().is_empty() {
            assert_eq!(wanted.len(), 4, "{wanted:?}");
        } else {
            assert!(wanted.is_empty(), "{wanted:?}");
        }
    }
}
