//! **The geology key's boot, its pin and the disabled state** (doc 20 cut SM.1; §3.2, Q-20-12,
//! §16).
//!
//! The host loads the geology key (`mush_config::geology_keys`) and hands it to this layer after
//! composition and before the module boot pass ([`super::install_geology`], the post-composition
//! device `mush_server::boot::build_engine` already uses for the ground cell size). The boot
//! pass (`boot`, called from `TacsimRuleset::on_world_loaded`) then decides, once:
//!
//! - **the first keyed boot pins** the key's fingerprint (SHA-256 of its 32 bytes) in the
//!   module document [`GEOLOGY_PIN_KEY`] — a boot-time module document on
//!   `store::put_mind_source`'s precedent, never an authoritative submission;
//! - **every later boot compares** it: the same fingerprint and version is enabled; anything
//!   else leaves a RAM-only disabled state and writes nothing.
//!
//! **A lost key is a disabled layer, not a corrupted one** (§16): a key that will not load, a
//! fingerprint that differs, an unreadable pin, a layer that is off and a key directory that is
//! not configured each leave [`GeologyDisabled`] with its reason, which the readiness report's
//! `geology:` line prints (`geology_line`; reported, never gated, in D-18-50's sky manner) and
//! which every later consumer (SM.4's instruments, SM.6's opener, SM.7's drill) refuses on
//! through [`live`]. The status is computed here and never recomputed after boot.

use mush_core::{Ctx, GeologyMac, World};
use serde::{Deserialize, Serialize};

use super::GeologyState;
use super::reading::READING_RULE_VERSION;
use crate::arena::{digest_from_text, digest_text};
use crate::port::{Binding, LineState, ReadinessLine};
use crate::store::TacsimStore;

/// The world's pin of the geology key's fingerprint (doc 20 cut SM.1; Q-20-12).
pub const GEOLOGY_PIN_KEY: &str = "tacsim:geology";

/// What the host hands the layer (the SM.1 brief's lead ruling 3).
pub use mush_core::GeologyBoot;

/// Why the geology layer is disabled. RAM only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeologyDisabled {
    /// Before `on_world_loaded`.
    NotBooted,
    /// `[tacsim].enabled = false`.
    LayerOff,
    /// No `[tacsim.geology].key_dir`.
    NotConfigured,
    /// The key store's error text (never key bytes).
    KeyUnreadable(String),
    /// `tacsim:geology` will not parse.
    PinUnreadable,
    /// The loaded key is not the pinned one: both 64-character base-16 fingerprints (or the
    /// pinned version differs).
    Mismatch {
        /// The pinned fingerprint.
        pinned: String,
        /// The loaded key's fingerprint.
        loaded: String,
    },
}

impl GeologyDisabled {
    /// The reason, as staff read it on the readiness line and from `@geology`.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            GeologyDisabled::NotBooted => "not booted yet".to_owned(),
            GeologyDisabled::LayerOff => "the layer is off".to_owned(),
            GeologyDisabled::NotConfigured => {
                "no [tacsim.geology].key_dir is configured".to_owned()
            }
            GeologyDisabled::KeyUnreadable(error) => {
                format!("the key directory did not load ({error})")
            }
            GeologyDisabled::PinUnreadable => {
                "tacsim:geology cannot be read; staff repair the document".to_owned()
            }
            GeologyDisabled::Mismatch { pinned, loaded } => format!(
                "the key's fingerprint {} is not the pinned {}; restore the key directory this \
                 world first booted with",
                prefix(loaded),
                prefix(pinned)
            ),
        }
    }
}

/// A fingerprint's first twelve characters, never by slicing.
fn prefix(text: &str) -> String {
    text.chars().take(12).collect()
}

/// The geology layer's state, decided once at boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeologyStatus {
    /// The pinned key is loaded.
    Enabled {
        /// Its version.
        version: u32,
        /// Its fingerprint.
        fingerprint: [u8; 32],
    },
    /// Disabled, and why.
    Disabled(GeologyDisabled),
}

/// The pin document's body (`tacsim:geology`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeologyPinRecord {
    /// The key's fingerprint, 64 lowercase base-16 characters.
    pub fingerprint: String,
    /// The key's version.
    pub key_version: u32,
}

/// What the store loaded under [`GEOLOGY_PIN_KEY`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GeologyPinState {
    /// No document: the next keyed boot pins.
    #[default]
    Absent,
    /// A readable pin.
    Present(GeologyPinRecord),
    /// A document that will not parse (the R8 rule: the key still bars a rewrite).
    Unreadable,
}

/// One `tacsim:geology` body: a record whose fingerprint is 64 lowercase base-16 characters and
/// whose version is not 0, else unreadable — logged naming the key, and never rewritten.
pub(crate) fn load_geology_pin(key: &str, value: &serde_json::Value) -> GeologyPinState {
    match serde_json::from_value::<GeologyPinRecord>(value.clone()) {
        Ok(record)
            if digest_from_text(&record.fingerprint).is_some() && record.key_version != 0 =>
        {
            GeologyPinState::Present(record)
        }
        Ok(_) => {
            tracing::warn!(
                %key,
                "tacsim: the geology pin's fingerprint or version is malformed; the mining layer \
                 reads disabled until staff repair the document"
            );
            GeologyPinState::Unreadable
        }
        Err(error) => {
            tracing::warn!(
                %key, %error,
                "tacsim: the geology pin did not parse; the mining layer reads disabled until \
                 staff repair the document"
            );
            GeologyPinState::Unreadable
        }
    }
}

/// The one live handle every consumer takes (SM.4, SM.6, SM.7): the keyed state and its
/// version, or the reason the layer is disabled.
///
/// # Errors
///
/// The [`GeologyDisabled`] reason (`NotBooted` when no geology slot is composed).
pub fn live(w: &World) -> Result<(GeologyMac, u32), GeologyDisabled> {
    let Some(state) = w.module_state::<GeologyState>() else {
        return Err(GeologyDisabled::NotBooted);
    };
    match (&state.status, &state.mac) {
        (GeologyStatus::Enabled { version, .. }, Some(mac)) => Ok((mac.clone(), *version)),
        (GeologyStatus::Disabled(disabled), _) => Err(disabled.clone()),
        // Enabled with no keyed state cannot be built by `boot`; refused as not booted.
        (GeologyStatus::Enabled { .. }, None) => Err(GeologyDisabled::NotBooted),
    }
}

/// The geology status, if a slot is composed.
#[must_use]
pub fn status(w: &World) -> Option<&GeologyStatus> {
    w.module_state::<GeologyState>().map(|state| &state.status)
}

/// What the boot pass decides: the status, the keyed state when enabled, and the pin to write.
struct Decision {
    status: GeologyStatus,
    mac: Option<GeologyMac>,
    write: Option<GeologyPinRecord>,
}

/// The pure half of [`boot`]: the layer's flag, the handed-in key and the loaded pin to a
/// status, logging each outcome once.
fn decide(enabled: bool, boot: Option<GeologyBoot>, pin: &GeologyPinState) -> Decision {
    let disabled = |reason: GeologyDisabled| Decision {
        status: GeologyStatus::Disabled(reason),
        mac: None,
        write: None,
    };
    if !enabled {
        return disabled(GeologyDisabled::LayerOff);
    }
    let secret = match boot {
        None | Some(GeologyBoot::NotConfigured) => {
            tracing::warn!(
                "tacsim: no [tacsim.geology].key_dir is configured; the mining layer is disabled"
            );
            return disabled(GeologyDisabled::NotConfigured);
        }
        Some(GeologyBoot::Unreadable(error)) => {
            tracing::error!(
                %error,
                "tacsim: the geology key directory did not load; the mining layer is disabled"
            );
            return disabled(GeologyDisabled::KeyUnreadable(error));
        }
        Some(GeologyBoot::Key(secret)) => secret,
    };
    let fingerprint = secret.fingerprint();
    let text = digest_text(&fingerprint);
    let version = secret.version();
    let enabled = |write: Option<GeologyPinRecord>| Decision {
        status: GeologyStatus::Enabled {
            version,
            fingerprint,
        },
        mac: Some(secret.mac()),
        write,
    };
    match pin {
        GeologyPinState::Absent => {
            tracing::info!(
                version,
                fingerprint = %prefix(&text),
                "geology key pinned"
            );
            enabled(Some(GeologyPinRecord {
                fingerprint: text,
                key_version: version,
            }))
        }
        GeologyPinState::Present(record)
            if record.fingerprint == text && record.key_version == version =>
        {
            tracing::info!(
                version,
                fingerprint = %prefix(&text),
                "geology key matches its pin"
            );
            enabled(None)
        }
        GeologyPinState::Present(record) => {
            tracing::error!(
                pinned = %prefix(&record.fingerprint),
                pinned_version = record.key_version,
                loaded = %prefix(&text),
                loaded_version = version,
                "tacsim: the geology key is not the one this world pinned; the mining layer is \
                 disabled until the key directory this world first booted with is restored"
            );
            disabled(GeologyDisabled::Mismatch {
                pinned: record.fingerprint.clone(),
                loaded: text,
            })
        }
        GeologyPinState::Unreadable => {
            tracing::error!(
                "tacsim: tacsim:geology cannot be read; the mining layer is disabled until staff \
                 repair the document"
            );
            disabled(GeologyDisabled::PinUnreadable)
        }
    }
}

/// **The boot pass** (called from `TacsimRuleset::on_world_loaded` right after the store is
/// loaded): decide the status once, keep the keyed state, and — at the first keyed boot only —
/// write the pin. The handed-in key is taken out of the slot; only its keyed HMAC state stays.
pub(crate) fn boot(ctx: &mut Ctx<'_>) {
    let (enabled, pin) = match ctx.w.module_state::<TacsimStore>() {
        Some(store) => (store.enabled(), store.geology_pin.clone()),
        None => (false, GeologyPinState::Absent),
    };
    let Some(state) = ctx.w.module_state_mut::<GeologyState>() else {
        return;
    };
    let handed = state.boot.take();
    let decision = decide(enabled, handed, &pin);
    state.status = decision.status;
    state.mac = decision.mac;
    if let Some(record) = decision.write {
        crate::store::put_geology_pin(ctx, &record);
    }
}

/// **The readiness line** (`Binding::Geology`): Ready with the key's version and fingerprint
/// prefix; Missing before boot or with no slot; Flagged with the reason otherwise. Reported,
/// never gated (`crate::port::coop::gated`).
#[must_use]
pub(crate) fn geology_line(w: &World) -> ReadinessLine {
    let (state, text) = match status(w) {
        None => (
            LineState::Missing,
            "geology: disabled: not composed.".to_owned(),
        ),
        Some(GeologyStatus::Enabled {
            version,
            fingerprint,
        }) => (
            LineState::Ready,
            format!(
                "geology: key v{version}, fingerprint {}, pinned; reading rule \
                 v{READING_RULE_VERSION}.",
                prefix(&digest_text(fingerprint))
            ),
        ),
        Some(GeologyStatus::Disabled(GeologyDisabled::NotBooted)) => (
            LineState::Missing,
            "geology: disabled: not booted yet.".to_owned(),
        ),
        Some(GeologyStatus::Disabled(disabled)) => (
            LineState::Flagged,
            format!("geology: disabled: {}.", disabled.reason()),
        ),
    };
    ReadinessLine {
        port: None,
        binding: Binding::Geology,
        state,
        text,
    }
}
