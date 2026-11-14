//! [`checked!`](macro@crate::checked): the one strict wire macro (doc 46 §5.2, unit CT.1).
//!
//! Mind commerce stores its records as JSON and trusts none of it on the way back in. A record
//! built by `checked!` is two structs. The **domain struct** is the one the code holds; it
//! derives `Serialize` and nothing else of serde's. The **wire struct** is private, has the same
//! fields in the same order, and derives `Deserialize` with `deny_unknown_fields`. The domain
//! struct's `Deserialize` decodes the wire struct, moves the fields across, and then calls the
//! domain struct's own `validate`, so a value that exists has been validated whichever decoder
//! built it, `serde_json::from_slice` included.
//!
//! Until CT.1 the macro was pasted into twenty files of `mush-economy`, `mush-roll` and
//! `mush-market` under five names. The copies differed in three ways, and a call says each
//! of them in its head, since two of the three decide stored bytes or what compiles:
//!
//! - who sees the domain struct (`pub struct`, `pub(crate) struct`);
//! - who sees its fields (`pub fields`, `pub(crate) fields`, `pub(super) fields`,
//!   `private fields`);
//! - where a field's attributes land: nowhere (the head has no `attrs on` clause, and a field
//!   attribute does not compile), on the wire struct only (`attrs on wire`: they shape the
//!   decode and never the encode), or on both structs (`attrs on both`: a
//!   `skip_serializing_if` then changes the stored bytes).
//!
//! The macro adds nothing a copy did not do and takes no option a copy did not have. Pairs that
//! were written by hand (a parents list capped at three, a funding receipt) stay written by
//! hand.

/// Builds a domain struct, its private `deny_unknown_fields` wire struct, and a `Deserialize`
/// for the domain struct that validates what it decoded.
///
/// ```
/// mush_core::checked!(pub struct Fee, FeeWire, pub fields, attrs on wire, {
///     format: u16,
///     #[serde(default)]
///     minor: u64,
/// });
/// impl Fee {
///     fn validate(&self) -> Result<(), String> {
///         if self.format == 1 { Ok(()) } else { Err("fee format".into()) }
///     }
/// }
///
/// let fee: Fee = serde_json::from_str(r#"{"format":1}"#).unwrap();
/// assert_eq!(fee, Fee { format: 1, minor: 0 });
/// // The encode is the domain struct's own, in the order the fields are written.
/// assert_eq!(serde_json::to_string(&fee).unwrap(), r#"{"format":1,"minor":0}"#);
/// // A field the record does not have is refused, and so is a value that fails `validate`.
/// assert!(serde_json::from_str::<Fee>(r#"{"format":1,"minor":0,"tip":1}"#).is_err());
/// assert!(serde_json::from_str::<Fee>(r#"{"format":2}"#).is_err());
/// ```
///
/// # The head
///
/// `checked!(VIS struct Name, NameWire, FIELDS fields[, attrs on wire | attrs on both], { … })`
///
/// - `VIS` is the domain struct's visibility. The wire struct is always private to the module
///   of the call. Its name is part of the record's refusals (`serde` names the struct it
///   expected), so a call keeps the name it has.
/// - `FIELDS` is `pub`, `pub(crate)`, `pub(super)` (any `pub(…)`), or `private`, and applies to
///   every field of the domain struct.
/// - With no `attrs on` clause the fields take no attribute at all. `attrs on wire` puts each
///   field's attributes on the wire struct only; `attrs on both` puts them on both structs.
///   Only `attrs on both` can change what is written: under `attrs on wire` a
///   `skip_serializing_if` is silently without effect, which is what the copies that placed
///   attributes there did.
///
/// A field attribute under a head that has no `attrs on` clause does not compile:
///
/// ```compile_fail
/// mush_core::checked!(pub struct Fee, FeeWire, pub fields, {
///     format: u16,
///     #[serde(default)]
///     minor: u64,
/// });
/// impl Fee {
///     fn validate(&self) -> Result<(), String> { Ok(()) }
/// }
/// ```
///
/// # What the caller supplies
///
/// - `fn validate(&self) -> Result<(), E>` on the domain struct, `E: Display`. Its text is the
///   refusal a decode answers with.
/// - The field types' own `Clone`, `Debug`, `PartialEq`, `Eq`, `Serialize` and `Deserialize`.
/// - `serde` as a dependency of the calling crate: the expansion names `::serde`.
///
/// # What must not be changed here (doc 46 Q-46-2)
///
/// The order of the fields is the order of the stored bytes, and economy's consent hash and
/// plan hash are taken over those bytes. `deny_unknown_fields` and the call to `validate` are
/// what every record's two refusals rest on. The snapshot tests named `ct0_*` in the three
/// commerce crates pin all of it for each record built here.
#[macro_export]
macro_rules! checked {
    // The head, by who sees the fields. Each arm hands the rest on with the field visibility
    // as one bracketed group.
    ($svis:vis struct $name:ident, $wire:ident, pub fields, $($rest:tt)*) => {
        $crate::checked!(@place [pub] $svis struct $name, $wire, $($rest)*);
    };
    ($svis:vis struct $name:ident, $wire:ident, pub($($scope:tt)+) fields, $($rest:tt)*) => {
        $crate::checked!(@place [pub($($scope)+)] $svis struct $name, $wire, $($rest)*);
    };
    ($svis:vis struct $name:ident, $wire:ident, private fields, $($rest:tt)*) => {
        $crate::checked!(@place [] $svis struct $name, $wire, $($rest)*);
    };

    // Where a field's attributes land. Each arm writes a field as
    // `[visibility] [the domain struct's attributes] [the wire struct's attributes] name: type,`.
    //
    // Nowhere: the fields take none.
    (@place $fvis:tt $svis:vis struct $name:ident, $wire:ident, {
        $($field:ident : $ty:ty),* $(,)?
    }) => {
        $crate::checked!(@build $svis struct $name, $wire, {
            $($fvis [] [] $field: $ty,)*
        });
    };
    // On the wire struct only.
    (@place $fvis:tt $svis:vis struct $name:ident, $wire:ident, attrs on wire, {
        $($(#[$attr:meta])* $field:ident : $ty:ty),* $(,)?
    }) => {
        $crate::checked!(@build $svis struct $name, $wire, {
            $($fvis [] [$(#[$attr])*] $field: $ty,)*
        });
    };
    // On both structs.
    (@place $fvis:tt $svis:vis struct $name:ident, $wire:ident, attrs on both, {
        $($(#[$attr:meta])* $field:ident : $ty:ty),* $(,)?
    }) => {
        $crate::checked!(@build $svis struct $name, $wire, {
            $($fvis [$(#[$attr])*] [$(#[$attr])*] $field: $ty,)*
        });
    };

    // The two structs and the validating decode. Both structs list the fields in the order
    // the call wrote them.
    (@build $svis:vis struct $name:ident, $wire:ident, {
        $([$($fvis:tt)*] [$(#[$own:meta])*] [$(#[$decode:meta])*] $field:ident : $ty:ty,)*
    }) => {
        #[derive(Clone, Debug, PartialEq, Eq, ::serde::Serialize)]
        $svis struct $name {
            $($(#[$own])* $($fvis)* $field: $ty),*
        }
        #[derive(::serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire {
            $($(#[$decode])* $field: $ty),*
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                d: D,
            ) -> ::core::result::Result<Self, D::Error> {
                let wire = <$wire as ::serde::Deserialize<'de>>::deserialize(d)?;
                let value = Self { $($field: wire.$field),* };
                value
                    .validate()
                    .map_err(<D::Error as ::serde::de::Error>::custom)?;
                ::core::result::Result::Ok(value)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Deserializer};

    /// What a decode of `json` is refused with, without `serde_json`'s line and column.
    fn refusal<T: for<'de> Deserialize<'de> + std::fmt::Debug>(json: &str) -> String {
        let text = serde_json::from_str::<T>(json).unwrap_err().to_string();
        match text.split_once(" at line ") {
            Some((reason, _)) => reason.to_owned(),
            None => text,
        }
    }

    fn odd(value: u8) -> Result<(), String> {
        if value % 2 == 1 {
            Ok(())
        } else {
            Err("the value must be odd".into())
        }
    }

    /// `Some` only: an absent field is the default, an explicit null is refused.
    fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u8>, D::Error> {
        u8::deserialize(d).map(Some)
    }

    // No field attribute; fields the crate sees.
    checked!(pub(crate) struct Plain, PlainWire, pub(crate) fields, {
        zulu: u8,
        alpha: u8,
    });
    impl Plain {
        fn validate(&self) -> Result<(), String> {
            odd(self.zulu)
        }
    }

    // Attributes on the wire struct only: they shape the decode and never the encode.
    checked!(pub struct OnWire, OnWireWire, pub fields, attrs on wire, {
        zulu: u8,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
        alpha: Option<u8>,
    });
    impl OnWire {
        fn validate(&self) -> Result<(), String> {
            odd(self.zulu)
        }
    }

    // The same record with the attributes on both structs: now they shape the encode too.
    checked!(pub struct OnBoth, OnBothWire, pub fields, attrs on both, {
        zulu: u8,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
        alpha: Option<u8>,
    });
    impl OnBoth {
        fn validate(&self) -> Result<(), String> {
            odd(self.zulu)
        }
    }

    mod inner {
        // Fields the parent module sees, and no further.
        checked!(pub struct ForParent, ForParentWire, pub(super) fields, { zulu: u8, alpha: u8 });
        impl ForParent {
            pub(super) fn validate(&self) -> Result<(), String> {
                super::odd(self.zulu)
            }
        }

        // Fields nobody outside this module sees, and a struct private to it.
        checked!(struct Hidden, HiddenWire, private fields, attrs on wire, {
            #[serde(default)]
            zulu: u8,
        });
        impl Hidden {
            fn validate(&self) -> Result<(), String> {
                super::odd(self.zulu)
            }
        }

        pub(super) fn hidden_default_is_refused() -> String {
            super::refusal::<Hidden>("{}")
        }
        pub(super) fn hidden_round_trip() -> String {
            let hidden: Hidden = serde_json::from_str(r#"{"zulu":3}"#).unwrap();
            assert_eq!(hidden, Hidden { zulu: 3 });
            serde_json::to_string(&hidden).unwrap()
        }
    }

    #[test]
    fn the_encode_is_in_the_order_the_fields_are_written() {
        // Not alphabetical, and not the wire struct's doing: the domain struct encodes itself.
        let plain = Plain { zulu: 1, alpha: 2 };
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"zulu":1,"alpha":2}"#
        );
        // A decode takes the fields in any order and builds the same value.
        let back: Plain = serde_json::from_str(r#"{"alpha":2,"zulu":1}"#).unwrap();
        assert_eq!(back, plain);
    }

    #[test]
    fn an_unknown_field_is_refused() {
        assert_eq!(
            refusal::<Plain>(r#"{"zulu":1,"alpha":2,"extra":3}"#),
            "unknown field `extra`, expected `zulu` or `alpha`"
        );
        assert_eq!(
            refusal::<OnWire>(r#"{"zulu":1,"extra":3}"#),
            "unknown field `extra`, expected `zulu` or `alpha`"
        );
        assert_eq!(
            refusal::<OnBoth>(r#"{"zulu":1,"extra":3}"#),
            "unknown field `extra`, expected `zulu` or `alpha`"
        );
    }

    #[test]
    fn a_missing_field_is_refused_and_names_the_wire_struct_on_a_wrong_shape() {
        assert_eq!(refusal::<Plain>(r#"{"zulu":1}"#), "missing field `alpha`");
        assert_eq!(
            refusal::<Plain>("7"),
            "invalid type: integer `7`, expected struct PlainWire"
        );
    }

    #[test]
    fn a_failed_validation_is_refused_with_its_own_text() {
        assert_eq!(
            refusal::<Plain>(r#"{"zulu":2,"alpha":2}"#),
            "the value must be odd"
        );
        assert_eq!(
            refusal::<OnWire>(r#"{"zulu":2,"alpha":2}"#),
            "the value must be odd"
        );
        assert_eq!(refusal::<OnBoth>(r#"{"zulu":2}"#), "the value must be odd");
        assert_eq!(
            refusal::<inner::ForParent>(r#"{"zulu":2,"alpha":2}"#),
            "the value must be odd"
        );
        // A decode-side default does not get past validation either.
        assert_eq!(inner::hidden_default_is_refused(), "the value must be odd");
    }

    #[test]
    fn attributes_on_the_wire_shape_the_decode_and_never_the_encode() {
        // The decode honours `default` and `deserialize_with`.
        let absent: OnWire = serde_json::from_str(r#"{"zulu":1}"#).unwrap();
        assert_eq!(
            absent,
            OnWire {
                zulu: 1,
                alpha: None
            }
        );
        assert_eq!(
            refusal::<OnWire>(r#"{"zulu":1,"alpha":null}"#),
            "invalid type: null, expected u8"
        );
        // The encode does not know `skip_serializing_if`: the field is written as null.
        assert_eq!(
            serde_json::to_string(&absent).unwrap(),
            r#"{"zulu":1,"alpha":null}"#
        );
    }

    #[test]
    fn attributes_on_both_shape_the_decode_and_the_encode() {
        let absent: OnBoth = serde_json::from_str(r#"{"zulu":1}"#).unwrap();
        assert_eq!(
            absent,
            OnBoth {
                zulu: 1,
                alpha: None
            }
        );
        assert_eq!(
            refusal::<OnBoth>(r#"{"zulu":1,"alpha":null}"#),
            "invalid type: null, expected u8"
        );
        // Here `skip_serializing_if` is on the domain struct, so the absent field is not written.
        assert_eq!(serde_json::to_string(&absent).unwrap(), r#"{"zulu":1}"#);
        let present = OnBoth {
            zulu: 1,
            alpha: Some(4),
        };
        assert_eq!(
            serde_json::to_string(&present).unwrap(),
            r#"{"zulu":1,"alpha":4}"#
        );
    }

    #[test]
    fn field_visibility_is_the_head_s() {
        // `pub(super)` fields are this module's to read and to build.
        let value = inner::ForParent { zulu: 1, alpha: 2 };
        assert_eq!(value.zulu + value.alpha, 3);
        assert_eq!(inner::hidden_round_trip(), r#"{"zulu":3}"#);
    }
}
