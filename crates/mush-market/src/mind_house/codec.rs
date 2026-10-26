use serde::{Deserialize, Deserializer, Serialize};

pub(super) trait Row: Serialize + for<'de> Deserialize<'de> {
    const FAMILY: &'static str;
    const MAX_BYTES: usize;
    fn key(&self) -> String;
    fn validate(&self) -> Result<(), String>;
}
pub(super) fn bounded<T: Serialize>(value: &T, cap: usize) -> Result<serde_json::Value, String> {
    let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    mush_core::canonical_json_bytes_with_limit(&value, cap).map_err(|e| e.to_string())?;
    Ok(value)
}
pub(super) fn digest<T: Serialize>(
    domain: &str,
    value: &T,
    cap: usize,
) -> Result<mush_core::DefinitionDigest, String> {
    mush_core::DefinitionDigest::for_canonical_value(domain, &bounded(value, cap)?)
        .map_err(|e| e.to_string())
}
pub(super) fn decode<T: Row>(bytes: &[u8]) -> Result<T, String> {
    if bytes.is_empty() || bytes.len() > T::MAX_BYTES {
        return Err("house record exceeds its bound".into());
    }
    let row: T = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    row.validate()?;
    Ok(row)
}
pub(super) fn required<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

/// Retained element count is bounded independently of the outer byte preflight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(super) struct Bounded<T, const N: usize>(pub(super) Vec<T>);
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Bounded<T, N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor<T, const N: usize>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Visitor<T, N> {
            type Value = Bounded<T, N>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "at most {N} entries")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut rows = Vec::with_capacity(N);
                while let Some(row) = seq.next_element()? {
                    if rows.len() == N {
                        return Err(serde::de::Error::custom("too many house entries"));
                    }
                    rows.push(row);
                }
                Ok(Bounded(rows))
            }
        }
        d.deserialize_seq(Visitor::<T, N>(std::marker::PhantomData))
    }
}
