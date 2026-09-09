//! Attributes: case-insensitive names, text/JSON values, and the per-object
//! attribute map.
//!
//! Attribute names are case-insensitive ("Desc" == "desc") but remember the
//! casing they were first written with for display. Names are namespaced by
//! convention with a single `:` separator: `"desc"`, `"wod:sheet"`,
//! `"agent:persona"` — see [`UncasedName::namespace`].

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use crate::obj::ObjId;

/// A case-insensitive attribute name.
///
/// Equality, ordering, and hashing use the case-folded form; `Display`
/// preserves the original casing. Small names are stored inline
/// ([`SmolStr`]), so clones are cheap.
///
/// ```
/// use mush_core::attr::UncasedName;
/// let a = UncasedName::from("Desc");
/// let b = UncasedName::from("DESC");
/// assert_eq!(a, b);
/// assert_eq!(a.to_string(), "Desc"); // original casing preserved
/// ```
#[derive(Clone)]
pub struct UncasedName {
    original: SmolStr,
    folded: SmolStr,
}

impl UncasedName {
    /// Build a name from any string, remembering its original casing.
    #[must_use]
    pub fn new(name: &str) -> UncasedName {
        UncasedName {
            original: SmolStr::new(name),
            folded: SmolStr::from(name.to_lowercase()),
        }
    }

    /// The name as originally written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.original
    }

    /// The case-folded (lowercase) form used for comparisons.
    #[must_use]
    pub fn folded(&self) -> &str {
        &self.folded
    }

    /// The namespace part of an `"ns:name"` attribute, if any
    /// (text before the first `:`, in original casing).
    ///
    /// ```
    /// use mush_core::attr::UncasedName;
    /// assert_eq!(UncasedName::from("wod:sheet").namespace(), Some("wod"));
    /// assert_eq!(UncasedName::from("desc").namespace(), None);
    /// ```
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.original.split_once(':').map(|(ns, _)| ns)
    }

    /// The name with any leading `"ns:"` namespace stripped
    /// (text after the first `:`; the whole name when un-namespaced).
    #[must_use]
    pub fn local_name(&self) -> &str {
        self.original
            .split_once(':')
            .map_or(self.original.as_str(), |(_, local)| local)
    }
}

impl PartialEq for UncasedName {
    fn eq(&self, other: &Self) -> bool {
        self.folded == other.folded
    }
}
impl Eq for UncasedName {}

impl PartialOrd for UncasedName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for UncasedName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.folded.cmp(&other.folded)
    }
}

impl Hash for UncasedName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.folded.hash(state);
    }
}

impl fmt::Display for UncasedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.original)
    }
}

impl fmt::Debug for UncasedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.original.as_str())
    }
}

impl From<&str> for UncasedName {
    fn from(s: &str) -> Self {
        UncasedName::new(s)
    }
}
impl From<String> for UncasedName {
    fn from(s: String) -> Self {
        UncasedName::new(&s)
    }
}
impl From<&SmolStr> for UncasedName {
    fn from(s: &SmolStr) -> Self {
        UncasedName::new(s)
    }
}

impl Serialize for UncasedName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.original)
    }
}

impl<'de> Deserialize<'de> for UncasedName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(UncasedName::new(&s))
    }
}

bitflags::bitflags! {
    /// Per-attribute permission/behavior flags.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
    pub struct AttrFlags: u8 {
        /// Only Wizard+ may read or write.
        const WIZARD_ONLY = 1 << 0;
        /// Only the object itself (and staff) may read.
        const SELF_ONLY   = 1 << 1;
        /// Not shown in normal `examine` output.
        const HIDDEN      = 1 << 2;
        /// May not be modified without unlocking (`@lock`ed attribute).
        const LOCKED      = 1 << 3;
        /// Not visible through the `@parent` inheritance chain.
        const NO_INHERIT  = 1 << 4;
    }
}

crate::flags::impl_bits_serde!(AttrFlags, u8);

impl AttrFlags {
    /// The set bits as their flag names, in bit order.
    ///
    /// `examine` renders these beside the setter and `@set/flag` lists them
    /// when it does not recognise a name; the parse in the other direction is
    /// `AttrFlags::from_name`, which bitflags derives (uppercase names only,
    /// so callers upcase first).
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        self.iter_names().map(|(n, _)| n).collect()
    }
}

/// A single attribute value: classic text, or structured JSON so rulesets
/// (sheets, agent config) don't have to serialize into strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AttrValue {
    /// Classic MUSH text attribute.
    Text(String),
    /// Structured payload (sheets: `sheet:wod`, agent config: `agent:*`).
    Json(serde_json::Value),
}

/// One attribute: value plus audit owner and flags.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attr {
    /// The stored value.
    pub value: AttrValue,
    /// Who set it (staff auditing), not who it is on.
    pub owner: ObjId,
    /// Permission/behavior flags.
    pub flags: AttrFlags,
}

impl Attr {
    /// A plain text attribute with empty flags.
    #[must_use]
    pub fn text(value: impl Into<String>, owner: ObjId) -> Attr {
        Attr {
            value: AttrValue::Text(value.into()),
            owner,
            flags: AttrFlags::empty(),
        }
    }

    /// A JSON attribute with empty flags.
    #[must_use]
    pub fn json(value: serde_json::Value, owner: ObjId) -> Attr {
        Attr {
            value: AttrValue::Json(value),
            owner,
            flags: AttrFlags::empty(),
        }
    }

    /// Borrow the text value, if this is a `Text` attribute.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match &self.value {
            AttrValue::Text(s) => Some(s),
            AttrValue::Json(_) => None,
        }
    }

    /// Borrow the JSON value, if this is a `Json` attribute.
    #[must_use]
    pub fn as_json(&self) -> Option<&serde_json::Value> {
        match &self.value {
            AttrValue::Text(_) => None,
            AttrValue::Json(v) => Some(v),
        }
    }
}

/// The per-object attribute map, keyed by case-insensitive name.
///
/// Serializes transparently as a map of original-cased name to attribute.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttrMap(BTreeMap<UncasedName, Attr>);

impl AttrMap {
    /// An empty map.
    #[must_use]
    pub fn new() -> AttrMap {
        AttrMap(BTreeMap::new())
    }

    /// Look up an attribute by name, case-insensitively.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Attr> {
        self.0.get(&UncasedName::new(name))
    }

    /// Look up by an already-folded key (avoids re-folding in hot walks).
    #[must_use]
    pub fn get_uncased(&self, key: &UncasedName) -> Option<&Attr> {
        self.0.get(key)
    }

    /// Mutable lookup by name, case-insensitively.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Attr> {
        self.0.get_mut(&UncasedName::new(name))
    }

    /// Set (insert or replace) an attribute, returning the previous value.
    ///
    /// The stored display casing is updated to the casing of `name`.
    pub fn set(&mut self, name: &str, attr: Attr) -> Option<Attr> {
        let key = UncasedName::new(name);
        let old = self.0.remove(&key);
        self.0.insert(key, attr);
        old
    }

    /// Remove an attribute by name, returning it if present.
    pub fn remove(&mut self, name: &str) -> Option<Attr> {
        self.0.remove(&UncasedName::new(name))
    }

    /// Whether an attribute with this (case-insensitive) name exists.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(&UncasedName::new(name))
    }

    /// Iterate attributes in folded-name order.
    pub fn iter(&self) -> impl Iterator<Item = (&UncasedName, &Attr)> {
        self.0.iter()
    }

    /// Iterate only the attributes in the given namespace
    /// (case-insensitive match on the `"ns:"` prefix).
    pub fn iter_namespace<'a>(
        &'a self,
        ns: &'a str,
    ) -> impl Iterator<Item = (&'a UncasedName, &'a Attr)> + 'a {
        self.0
            .iter()
            .filter(move |(k, _)| k.namespace().is_some_and(|n| n.eq_ignore_ascii_case(ns)))
    }

    /// Number of attributes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obj::GOD;

    #[test]
    fn uncased_eq_ord_hash_fold() {
        let a = UncasedName::from("Desc");
        let b = UncasedName::from("dEsC");
        assert_eq!(a, b);
        assert_eq!(a.cmp(&b), Ordering::Equal);
        let mut map = std::collections::HashMap::new();
        map.insert(a.clone(), 1);
        assert_eq!(map.get(&b), Some(&1));
        // Ord is by folded case: "Zebra" > "apple" when folded.
        assert!(UncasedName::from("apple") < UncasedName::from("Zebra"));
    }

    #[test]
    fn uncased_display_preserves_original() {
        let n = UncasedName::from("Wod:Sheet");
        assert_eq!(n.to_string(), "Wod:Sheet");
        assert_eq!(n.folded(), "wod:sheet");
    }

    #[test]
    fn namespace_helpers() {
        let n = UncasedName::from("wod:sheet");
        assert_eq!(n.namespace(), Some("wod"));
        assert_eq!(n.local_name(), "sheet");

        let plain = UncasedName::from("desc");
        assert_eq!(plain.namespace(), None);
        assert_eq!(plain.local_name(), "desc");

        // Nested colons: namespace is up to the FIRST colon.
        let nested = UncasedName::from("a:b:c");
        assert_eq!(nested.namespace(), Some("a"));
        assert_eq!(nested.local_name(), "b:c");
    }

    #[test]
    fn attrmap_case_insensitive_get_set_remove() {
        let mut m = AttrMap::new();
        assert!(m.set("Desc", Attr::text("A dusty room.", GOD)).is_none());
        assert_eq!(m.get("DESC").unwrap().as_text(), Some("A dusty room."));
        assert!(m.contains("desc"));
        assert_eq!(m.len(), 1);

        // Replacing under a different casing keeps one entry, latest casing.
        let old = m.set("DESC", Attr::text("Repainted.", GOD));
        assert_eq!(old.unwrap().as_text(), Some("A dusty room."));
        assert_eq!(m.len(), 1);
        assert_eq!(m.iter().next().unwrap().0.as_str(), "DESC");

        assert!(m.remove("dEsc").is_some());
        assert!(m.is_empty());
    }

    #[test]
    fn attrmap_namespace_iter() {
        let mut m = AttrMap::new();
        m.set("desc", Attr::text("x", GOD));
        m.set("wod:sheet", Attr::json(serde_json::json!({"str": 2}), GOD));
        m.set("WOD:clan", Attr::text("Ventrue", GOD));
        m.set("agent:persona", Attr::text("barkeep", GOD));
        let wod: Vec<_> = m.iter_namespace("wod").map(|(k, _)| k.as_str()).collect();
        assert_eq!(wod, vec!["WOD:clan", "wod:sheet"]);
    }

    #[test]
    fn attrmap_serde_round_trip_preserves_casing() {
        let mut m = AttrMap::new();
        m.set("Desc", Attr::text("hello", GOD));
        m.set(
            "wod:sheet",
            Attr {
                value: AttrValue::Json(serde_json::json!({"dots": 3})),
                owner: GOD,
                flags: AttrFlags::NO_INHERIT | AttrFlags::HIDDEN,
            },
        );
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            json.contains("\"Desc\""),
            "original casing on the wire: {json}"
        );
        let back: AttrMap = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
        assert_eq!(
            back.get("WOD:SHEET").unwrap().flags,
            AttrFlags::NO_INHERIT | AttrFlags::HIDDEN
        );
    }
}
