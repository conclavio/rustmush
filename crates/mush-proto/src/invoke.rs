//! Closed, bounded invokes. Mind authoring runs with owner mutation authority;
//! gateway legal offers enter the authoritative coordinator instead.

use serde::{Deserialize, Serialize};

/// Maximum complete JSON request or result, before deserialization.
pub const MAX_INVOKE_BYTES: usize = 272 * 1024;
/// Maximum encoded card metadata, excluding source and transport framing.
pub const MAX_MIND_ENVELOPE_BYTES: usize = 64 * 1024;
/// Maximum UTF-8 source bytes, independently of the whole envelope bound.
pub const MAX_MIND_SOURCE_BYTES: usize = 32 * 1024;
/// Maximum container nesting before parsing a request.
pub const MAX_INVOKE_DEPTH: usize = 32;

/// Finite invoke vocabulary. Only Mind is a module-owned service; legal offers
/// are gateway-owned and never dispatch a mutable module InvokeHandler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "service",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum InvokeRequest {
    /// Character-owned unit mind authoring.
    Mind(MindRequest),
    /// Gateway-owned opaque legal offer submission and authenticated status.
    LegalAction(crate::LegalActionRequest),
    /// Staff unit designer (doc 31 cut UD.2): a design's rules checked by the
    /// server's own loader and catalogue build.
    UnitDesign(UnitDesignRequest),
}
impl InvokeRequest {
    /// Registration key, independent of caller-controlled strings.
    #[must_use]
    pub const fn service(&self) -> &'static str {
        match self {
            Self::Mind(_) => "mind",
            Self::LegalAction(_) => "legal_action",
            Self::UnitDesign(_) => "unit_design",
        }
    }
    /// A request-family-correct transport refusal, without declaring an action absent.
    #[must_use]
    pub fn unavailable(&self) -> InvokeResult {
        match self {
            Self::Mind(_) => InvokeResult::error("unavailable", "This service is unavailable."),
            Self::UnitDesign(_) => InvokeResult::UnitDesign(UnitDesignResponse::Error {
                code: "unavailable".into(),
                message: "This service is unavailable.".into(),
            }),
            Self::LegalAction(_) => {
                InvokeResult::LegalAction(crate::LegalActionResponse::Unavailable {
                    reference: None,
                })
            }
        }
    }
    /// Preserve the request family when a decoded request fails its bounds.
    #[must_use]
    pub fn invalid(&self) -> InvokeResult {
        match self {
            Self::Mind(_) => {
                InvokeResult::error("invalid", "Invalid or oversized structured request.")
            }
            Self::UnitDesign(_) => InvokeResult::UnitDesign(UnitDesignResponse::Error {
                code: "invalid".into(),
                message: "Invalid or oversized structured request.".into(),
            }),
            Self::LegalAction(_) => {
                InvokeResult::LegalAction(crate::LegalActionResponse::Invalid {})
            }
        }
    }
    /// Validate the complete wire envelope and source bounds again at ingress.
    pub fn validate(&self) -> Result<(), &'static str> {
        let bytes = serde_json::to_vec(self).map_err(|_| "invalid request")?;
        validate_invoke_json(&bytes)?;
        let request = match self {
            Self::Mind(request) => request,
            Self::LegalAction(request) => return request.validate(),
            Self::UnitDesign(request) => return request.validate(),
        };
        if let MindRequest::FiledSource {
            licence,
            first,
            last,
        } = request
        {
            let raw = licence
                .parse::<u64>()
                .map_err(|_| "invalid paid source selector")?;
            if raw == 0
                || raw.to_string() != *licence
                || *first == 0
                || last < first
                || last - first >= 200
            {
                return Err("invalid paid source range or selector");
            }
        }
        if let MindRequest::Remove { name } = request
            && (name.is_empty() || name.len() > 64 || name.contains(['\0', '@', ':']))
        {
            return Err("invalid owned removal name");
        }
        // 17g cut C3: a typed market action's bounds (`MarketAction::validate`);
        // the code is already a `ListingCode`.
        if let MindRequest::Market { action } = request {
            action.validate()?;
        }
        if let MindRequest::Replace { draft, .. } = request {
            if draft.source.len() > MAX_MIND_SOURCE_BYTES || draft.source.contains('\0') {
                return Err("source exceeds 32 KiB or contains NUL");
            }
            let metadata = serde_json::json!({
                "schema": draft.schema, "presets": draft.presets,
                "profiles": draft.profiles, "rules": draft.rules,
                "declared_class": draft.declared_class, "mind_abi": draft.mind_abi
            });
            if serde_json::to_vec(&metadata)
                .map_err(|_| "invalid metadata")?
                .len()
                > MAX_MIND_ENVELOPE_BYTES
            {
                return Err("metadata envelope exceeds 64 KiB");
            }
        }
        Ok(())
    }
}

/// Compare-and-swap token returned by the owner; never a validation receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MindEditBase {
    /// Working-copy edit generation.
    pub edit: u64,
    /// Last saved local revision, if any.
    pub revision: Option<u64>,
}

/// Complete editable envelope. Game semantics are checked by the same owner
/// services as line commands; JSON is never evaluated as command text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MindDraft {
    /// Exact source, including line endings.
    pub source: String,
    /// Declarative card schema.
    pub schema: serde_json::Value,
    /// Named card presets.
    pub presets: serde_json::Value,
    /// Named profile values.
    pub profiles: serde_json::Value,
    /// Ordered rules by profile.
    pub rules: serde_json::Value,
    /// Declared core class.
    pub declared_class: String,
    /// Executable mind ABI.
    pub mind_abi: String,
}

/// Structured counterparts of the card/profile/rule editors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindEdit {
    /// Set one schema-defined field.
    Set {
        /// Profile name.
        profile: String,
        /// Field identifier.
        field: String,
        /// The schema's textual value syntax.
        value: String,
    },
    /// Save current profile values as a preset.
    PresetSave {
        /// Profile name.
        profile: String,
        /// Preset name.
        preset: String,
    },
    /// Apply a preset to one profile.
    PresetApply {
        /// Profile name.
        profile: String,
        /// Preset name.
        preset: String,
    },
    /// Remove one preset.
    PresetDelete {
        /// Preset name.
        preset: String,
    },
    /// Add a profile, optionally copying another.
    ProfileAdd {
        /// New name.
        name: String,
        /// Existing profile to copy.
        from: Option<String>,
    },
    /// Remove one profile.
    ProfileDelete {
        /// Profile name.
        name: String,
    },
    /// Rename one profile.
    ProfileRename {
        /// Existing name.
        old: String,
        /// New name.
        new: String,
    },
    /// Replace/remove an ordered rule using the native rule grammar.
    Rule {
        /// Profile name.
        profile: String,
        /// One-based authored slot.
        slot: u16,
        /// Empty removes; otherwise one bounded rule.
        text: String,
    },
}

/// Library lifecycle changes use the same exact draft token as working edits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindLifecycle {
    /// Rename this library entry without changing immutable revisions.
    Rename {
        /// New library name.
        new_name: String,
    },
    /// Remove the working entry; named and retained revisions remain rooted.
    // A tagged unit variant ignores unknown fields in serde; keep an empty
    // struct variant so the closed request grammar rejects extra fields.
    Remove {},
    /// Rename an existing local version label.
    VersionRename {
        /// Existing version label.
        old: String,
        /// New version label.
        new: String,
    },
    /// Release a version label into the bounded unnamed revision history.
    VersionForget {
        /// Version label to release.
        version: String,
    },
}

/// All requests act as the connection's selected character. No actor, account,
/// source path, validation receipt, or arbitrary command is accepted here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindRequest {
    /// Read a bounded current paid Open/OpenResale source range. This names no
    /// actor, form, account, revision override, or author-edit capability.
    FiledSource {
        /// Canonical nonzero decimal personal licence identity.
        licence: String,
        /// First source line, one-based.
        first: u32,
        /// Inclusive last line, at most 200 lines in one response.
        last: u32,
    },
    /// Request removal of an owned attributed working draft using source-free
    /// current metadata. The server captures its exact identity and edit base;
    /// originals keep the existing exact-CAS Lifecycle operation.
    Remove {
        /// Owned working library name, never an actor or revision selector.
        name: String,
    },
    /// Change a library entry or version label with exact edit authority.
    Lifecycle {
        /// Current library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
        /// Owner operation.
        edit: MindLifecycle,
    },
    /// Remove up to this many unreferenced revisions owned by this character.
    Prune {
        /// One through 128; all retained roots still apply.
        limit: u16,
    },
    /// Select an authorized own-hull trace feed; this is never source access.
    Trace {
        /// Hull object selector, resolved through the current trace ACL.
        unit: u64,
        /// One through 64 records; absent means three.
        count: Option<u8>,
        /// Keep following after this bounded read.
        follow: bool,
    },
    /// List this character's current library.
    List {},
    /// Read working copy (`None`) or one local revision/version/sha reference.
    Read {
        /// Library name.
        name: String,
        /// Local revision selector, without a name or owner.
        revision: Option<String>,
    },
    /// Create a new copy; `from` uses the existing owned-copy reference syntax.
    Create {
        /// New library name.
        name: String,
        /// Optional owned source or the public Standard template.
        from: Option<String>,
    },
    /// Replace the working copy. This does not save an immutable revision.
    Replace {
        /// Library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
        /// New complete working copy.
        draft: Box<MindDraft>,
    },
    /// Edit one card/profile/rule through the shared typed editor.
    Edit {
        /// Library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
        /// Typed edit.
        edit: MindEdit,
    },
    /// Check the exact working copy in the six isolated pictures, without save.
    Check {
        /// Library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
    },
    /// Validate then publish an immutable revision of this exact draft.
    Save {
        /// Library name.
        name: String,
        /// Exact current draft token.
        base: MindEditBase,
        /// Bounded revision notes.
        notes: String,
        /// Optional local version label.
        version: Option<String>,
    },
    /// Inspect a job submitted on this same connection lifetime.
    Status {
        /// Opaque queued job number.
        job: u64,
    },
    /// Cancel a job submitted on this same connection lifetime.
    Cancel {
        /// Opaque queued job number.
        job: u64,
    },
    /// 17g cut C3: a commerce action, typed. 17g cut C2's player market,
    /// holdings and trade commands one-for-one; the server selects the
    /// actor's own state from these references and builds every parameter.
    /// Nothing here names an account, a recipient's account, a token or a
    /// price the server derives.
    Market {
        /// The one action, as the line command's arguments name it.
        action: MarketAction,
    },
}

/// 17g cut C3: the thirteen player commands of 17g cut C2, typed. Each field
/// is the argument noun the line command takes (17g §1 ruling 4): a listing
/// code, a filed or licence number, a player name, a price in the line's
/// `price=` spelling, a form, a royalty and the author's transfer mark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "act", rename_all = "snake_case", deny_unknown_fields)]
pub enum MarketAction {
    /// `+minds/buy <code>`: ask for a quote on the current listing with this code.
    Quote {
        /// The listing's code, as `+minds` prints it.
        code: ListingCode,
    },
    /// `+minds/buy/pay <code>`: pay the actor's newest live quote on this listing into escrow.
    Pay {
        /// The listing's code.
        code: ListingCode,
    },
    /// `+minds/buy/complete <code>`: take the licence from the actor's funded purchase.
    Complete {
        /// The listing's code.
        code: ListingCode,
    },
    /// `+minds/buy/refund <code>`: ask for the actor's funded purchase back.
    Refund {
        /// The listing's code.
        code: ListingCode,
    },
    /// `+minds/renew <licence>`: hand the renewal's status, or a request for an offer, to the service.
    Renew {
        /// The licence number, as `+minds/holdings` prints it.
        licence: u64,
    },
    /// `+minds/renew/pay <licence>`: hand the payment of the drafted offer at its frozen price to the service.
    RenewPay {
        /// The licence number.
        licence: u64,
    },
    /// `+minds/renew/decline <licence>`: hand the decline of the drafted offer to the service.
    RenewDecline {
        /// The licence number.
        licence: u64,
    },
    /// `+minds/resell <licence>=<player> price=<credits>`: offer a held licence to a player.
    Resell {
        /// The licence number.
        licence: u64,
        /// The buyer as a player would type the name; the server resolves it.
        player: String,
        /// The price in credits, as `price=` spells it (`12.50`, `0.21cr`).
        price: String,
    },
    /// `+minds/resell/accept <licence> [price=<credits>]`: list the offers naming the actor
    /// (no price), or accept the one at exactly this price.
    ResellAccept {
        /// The licence number the seller named.
        licence: u64,
        /// The offer's price as `price=` spells it; absent lists the offers.
        price: Option<String>,
    },
    /// `+minds/charter <filed>=<player> price=<credits>`: offer a filed mind's charter to a player.
    Charter {
        /// The filed number, as `+minds/holdings` prints it.
        filed: u64,
        /// The buyer as a player would type the name; the server resolves it.
        player: String,
        /// The price in credits, as `price=` spells it.
        price: String,
    },
    /// `+minds/charter/accept <filed> [price=<credits>]`: as `ResellAccept`, for a charter.
    CharterAccept {
        /// The filed number the seller named.
        filed: u64,
        /// The offer's price as `price=` spells it; absent lists the offers.
        price: Option<String>,
    },
    /// `@mind/publish <filed> form=<form> [royalty=<per mille>] [transfer=<yes|no>]`.
    Publish {
        /// The filed number.
        filed: u64,
        /// The listing's form.
        form: MarketListingForm,
        /// Parent royalty in per mille, 0 to 300; absent is 0 (and a sealed listing takes 0).
        royalty: Option<u16>,
        /// The author's transfer mark (D-UM-40); absent is transferable.
        transfer: Option<bool>,
    },
    /// `@mind/withdraw <filed>`: stop new sales of a filed revision.
    Withdraw {
        /// The filed number.
        filed: u64,
    },
}

/// 17g cut C3: `@mind/publish`'s three `form=` words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum MarketListingForm {
    /// `form=sealed`.
    Sealed,
    /// `form=open`.
    Open,
    /// `form=open-resale`.
    OpenResale,
}

/// 17g cut C3: exactly the 8 characters of a listing's publication token
/// that `+minds` prints (each a digit 0-9 or a letter a-f,
/// `mush_market::mind_commands::is_code`); anything else is refused at
/// decode, before any read.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct ListingCode(String);

/// The number of characters of a listing code (`mush_market::mind_commands::
/// CODE_CHARACTERS`).
pub const LISTING_CODE_CHARACTERS: usize = 8;

impl ListingCode {
    /// The code text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for ListingCode {
    type Error = ListingCodeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() == LISTING_CODE_CHARACTERS
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(value))
        } else {
            Err(ListingCodeError)
        }
    }
}
impl TryFrom<&str> for ListingCode {
    type Error = ListingCodeError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}
impl From<ListingCode> for String {
    fn from(code: ListingCode) -> Self {
        code.0
    }
}
impl std::fmt::Display for ListingCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A listing code that is not 8 digits and letters a-f.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("a listing code is 8 characters, each a digit or a letter a to f")]
pub struct ListingCodeError;

/// The longest `player` a typed trade names (characters).
pub const MAX_MARKET_PLAYER_CHARS: usize = 32;
/// The longest `price` a typed trade or accept names (bytes).
pub const MAX_MARKET_PRICE_BYTES: usize = 16;
/// The largest parent royalty a typed listing names (per mille).
pub const MAX_MARKET_ROYALTY_PERMILLE: u16 = 300;

impl MarketAction {
    /// The bounds `InvokeRequest::validate` checks: no state is consulted;
    /// the grammar and the meaning are the server's (17g cut C2's words).
    fn validate(&self) -> Result<(), &'static str> {
        let player = |player: &str| {
            if player.is_empty()
                || player.chars().count() > MAX_MARKET_PLAYER_CHARS
                || player.chars().any(char::is_control)
            {
                Err("invalid market player")
            } else {
                Ok(())
            }
        };
        let price = |price: &str| {
            if price.is_empty()
                || price.len() > MAX_MARKET_PRICE_BYTES
                || !price
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'c' | b'r'))
            {
                Err("invalid market price")
            } else {
                Ok(())
            }
        };
        match self {
            Self::Resell {
                player: name,
                price: amount,
                ..
            }
            | Self::Charter {
                player: name,
                price: amount,
                ..
            } => {
                player(name)?;
                price(amount)
            }
            Self::ResellAccept {
                price: Some(amount),
                ..
            }
            | Self::CharterAccept {
                price: Some(amount),
                ..
            } => price(amount),
            Self::Publish {
                royalty: Some(royalty),
                ..
            } if *royalty > MAX_MARKET_ROYALTY_PERMILLE => Err("invalid market royalty"),
            _ => Ok(()),
        }
    }
}

/// A bounded diagnostic that never includes source excerpts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MindDiagnostic {
    /// Field or check stage.
    pub field: String,
    /// Short owner-approved explanation.
    pub message: String,
    /// Actual one-based parser line when available; absent means unknown.
    pub line: Option<u32>,
    /// Actual one-based parser column when available; absent means unknown.
    pub column: Option<u32>,
}
/// One actual isolated picture result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MindPictureReport {
    /// Stable picture identifier.
    pub picture: String,
    /// Whether this case had existing armings.
    pub armed: bool,
    /// Metered interpreter operations.
    pub operations: u64,
    /// Metered host fees.
    pub fees: u64,
    /// Bounded rendered intent descriptions.
    pub intents: Vec<String>,
    /// Coarse fault category, never raw source.
    pub fault: Option<String>,
    /// Actual station landing refusals.
    pub station_refusals: Vec<String>,
    /// Declared-class violations.
    pub class_violations: Vec<String>,
}
/// Final responses and explicit asynchronous progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum MindResponse {
    /// Current authorised paid source display. This is never an editable draft
    /// receipt or a reusable source-access capability.
    FiledSource {
        /// Canonical personal licence identity.
        licence: String,
        /// SHA256 of the exact complete immutable source.
        source_sha256: String,
        /// First requested one-based line.
        first: u32,
        /// Bounded numbered display lines, escaped by the shared source viewer.
        lines: Vec<String>,
    },
    /// The removal selector was requested. This is a final transport response,
    /// not a committed removal or a claim that durable intent is absent. Keep
    /// the entry until the actual receipt or current library confirms removal.
    RemovalRequested {
        /// Actual canonical owned label; no source material is returned.
        name: String,
    },
    /// The library entry was removed; retained immutable revisions survive.
    Removed {
        /// Former library name.
        name: String,
    },
    /// A bounded revision collection completed; source bytes remain retained.
    Pruned {
        /// Unreferenced revision documents removed.
        revisions: u16,
        /// Distinct sources of these owned revisions, all still retained.
        /// This count does not expose another character's source references.
        source_candidates_retained: u16,
    },
    /// Authorized bounded trace read and subscription cursor.
    Trace {
        /// Trace feed's source-free projection.
        data: serde_json::Value,
    },
    /// This character's bounded library labels.
    Library {
        /// Owned names.
        names: Vec<String>,
    },
    /// Exact source and envelope; token always describes the current draft.
    Read {
        /// Library name.
        name: String,
        /// Requested revision selector.
        revision: Option<String>,
        /// Current draft token.
        base: MindEditBase,
        /// Exact owned working copy or revision.
        draft: Box<MindDraft>,
        /// Canonical rule grammar by profile, rendered by the shared schema owner.
        rules_text: std::collections::BTreeMap<String, Vec<String>>,
        /// Exact editable grammar by profile/slot; null has no lossless spelling.
        rules_source: std::collections::BTreeMap<String, Vec<Option<String>>>,
        /// Invalid working metadata is reported without rewriting or hiding source.
        diagnostics: Vec<MindDiagnostic>,
    },
    /// A working edit completed; clients must use this new token next.
    Edited {
        /// Library name.
        name: String,
        /// New current draft token.
        base: MindEditBase,
    },
    /// Worker accepted; a final result follows with the original request id.
    Queued {
        /// Opaque job number.
        job: u64,
    },
    /// Explicit status poll; this is not a final result.
    Pending {
        /// Opaque job number.
        job: u64,
    },
    /// Check finished without saving.
    Checked {
        /// Opaque job number.
        job: u64,
        /// Six isolated cases, or no cases on compilation failure.
        reports: Vec<MindPictureReport>,
        /// Bounded diagnostics.
        diagnostics: Vec<MindDiagnostic>,
    },
    /// Immutable save completed through normal deferred persistence.
    Saved {
        /// Opaque job number.
        job: u64,
        /// Local revision number.
        revision: u64,
        /// Immutable executable definition digest.
        digest: String,
        /// New current draft token.
        base: MindEditBase,
    },
    /// Job cancelled; a running worker keeps its quota until it exits.
    Cancelled {
        /// Opaque job number.
        job: u64,
    },
    /// 17g cut C3: a typed market action was staged (or handed to the
    /// renewal service); the outcome arrives on the actor's ordinary output.
    /// This is not a success: the provider may still refuse.
    Staged {
        /// The request line the line command prints, byte for byte.
        line: String,
    },
    /// 17g cut C3: the server answered a typed market action with these
    /// lines and submitted nothing (an accept without a price).
    Shown {
        /// The lines the line command prints, in order.
        lines: Vec<String>,
    },
    /// Refusal; source ownership failures have the same bounded response.
    Error {
        /// Stable error category.
        code: String,
        /// Source-free explanation.
        message: String,
    },
}
impl MindResponse {
    /// Whether an SSH operation has reached its final result.
    #[must_use]
    pub const fn is_final(&self) -> bool {
        !matches!(self, Self::Queued { .. } | Self::Pending { .. })
    }
    /// Exit status reflects checks as well as request-level refusals.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        match self {
            Self::Error { .. } | Self::Cancelled { .. } => false,
            Self::Checked {
                reports,
                diagnostics,
                ..
            } => {
                diagnostics.is_empty()
                    && reports
                        .iter()
                        .all(|r| r.fault.is_none() && r.class_violations.is_empty())
            }
            _ => true,
        }
    }
}

/// Closed service reply, carried only to the requesting physical connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "service",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum InvokeResult {
    /// Character-owned unit mind service.
    Mind(MindResponse),
    /// Source-free admission/status result from the gateway-owned service.
    LegalAction(crate::LegalActionResponse),
    /// Staff unit designer result (doc 31 cut UD.2).
    UnitDesign(UnitDesignResponse),
}
impl InvokeResult {
    /// Generic ingress/registration refusal in the current service family.
    #[must_use]
    pub fn error(code: &str, message: &str) -> Self {
        Self::Mind(MindResponse::Error {
            code: code.into(),
            message: message.into(),
        })
    }
    /// Enforce response bounds before any output is queued.
    #[must_use]
    pub fn bounded(self) -> Self {
        if serde_json::to_vec(&self).is_ok_and(|v| validate_invoke_json(&v).is_ok()) {
            self
        } else {
            match self {
                Self::Mind(_) => {
                    Self::error("too_large", "Result exceeds the structured response limit.")
                }
                Self::LegalAction(_) => {
                    Self::LegalAction(crate::LegalActionResponse::Unavailable { reference: None })
                }
                Self::UnitDesign(_) => Self::UnitDesign(UnitDesignResponse::Error {
                    code: "too_large".into(),
                    message: "Result exceeds the structured response limit.".into(),
                }),
            }
        }
    }
}

/// The longest design id (`tacsim.unit.<slug>`), bytes.
pub const MAX_UNIT_DESIGN_ID_BYTES: usize = 96;
/// The largest design `rules` or `look`, encoded, bytes.
pub const MAX_UNIT_DESIGN_BYTES: usize = 128 * 1024;

/// **The staff unit designer's requests** (doc 31 cut UD.2, D-31-7 as amended:
/// the rules are checked on the server, by the loader and catalogue build the
/// game itself runs, never by a second implementation in the browser).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnitDesignRequest {
    /// Check a design and, when it holds, answer its finished rules pack.
    Check {
        /// The unit's definition id, `tacsim.unit.<slug>`.
        id: String,
        /// The pack's version, from 1.
        version: u32,
        /// The unit's `def` object, as `crates/tacsim/content/units/*.json`
        /// carries it under its entry.
        rules: serde_json::Value,
    },
    /// Publish a design to the running instance (doc 31 cut UD.3, D-31-9):
    /// checked as [`UnitDesignRequest::Check`] is, then kept as a registry
    /// row; fieldable after the next restart (Q-31-1). A published version
    /// never changes.
    Publish {
        /// The unit's definition id, `tacsim.unit.<slug>`.
        id: String,
        /// The pack's version, from 1.
        version: u32,
        /// The unit's `def` object.
        rules: serde_json::Value,
        /// Its look, as `crates/mush-pixel/looks/*.json` carries one.
        look: serde_json::Value,
    },
}

impl UnitDesignRequest {
    /// The request's own bounds, before the owner reads it.
    pub fn validate(&self) -> Result<(), &'static str> {
        let (id, version, rules) = match self {
            Self::Check { id, version, rules } => (id, version, rules),
            Self::Publish {
                id,
                version,
                rules,
                look,
            } => {
                if !look.is_object()
                    || serde_json::to_vec(look).map_err(|_| "invalid look")?.len()
                        > MAX_UNIT_DESIGN_BYTES
                {
                    return Err("a design's look is an object of at most 128 KiB");
                }
                (id, version, rules)
            }
        };
        if id.is_empty() || id.len() > MAX_UNIT_DESIGN_ID_BYTES {
            return Err("invalid design id");
        }
        if *version == 0 {
            return Err("a design's version is 1 or more");
        }
        if !rules.is_object()
            || serde_json::to_vec(rules)
                .map_err(|_| "invalid rules")?
                .len()
                > MAX_UNIT_DESIGN_BYTES
        {
            return Err("a design's rules are an object of at most 128 KiB");
        }
        Ok(())
    }
}

/// **The staff unit designer's answers.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnitDesignResponse {
    /// The design was checked. `refusals` is empty when it holds; then `pack`
    /// is its finished rules pack — the manifest's `requires` and `digest`
    /// filled in — ready for `crates/tacsim/content/units/`.
    Checked {
        /// Each reason the server's loader or catalogue build refused it.
        refusals: Vec<String>,
        /// The finished pack, when it holds.
        pack: Option<serde_json::Value>,
    },
    /// The design was published (doc 31 cut UD.3): its registry row is
    /// written, and it is fieldable after the next server restart.
    Published {
        /// The unit's definition id.
        id: String,
        /// The pack's version.
        version: u32,
        /// The refusals when it was not published (empty when it was).
        refusals: Vec<String>,
    },
    /// The request was not answered (not staff, unavailable, too large).
    Error {
        /// A short code.
        code: String,
        /// The sentence to show.
        message: String,
    },
}

/// Bounded linear preflight before serde allocates containers. This is not a
/// second JSON parser: serde remains responsible for all syntax validation.
pub fn validate_invoke_json(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() > MAX_INVOKE_BYTES {
        return Err("request exceeds 272 KiB");
    }
    let (mut depth, mut string, mut escaped) = (0_usize, false, false);
    for &byte in bytes {
        if string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                string = false;
            }
        } else {
            match byte {
                b'"' => string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_INVOKE_DEPTH {
                        return Err("request nesting exceeds 32");
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn paid_source_wire_is_closed_bounded_and_distinct_from_author_edits() {
        let request = super::InvokeRequest::Mind(super::MindRequest::FiledSource {
            licence: "18446744073709551615".into(),
            first: 1,
            last: 200,
        });
        request.validate().expect("bounded licence request");
        let bytes = serde_json::to_vec(&request).expect("wire");
        assert_eq!(
            serde_json::from_slice::<super::InvokeRequest>(&bytes).expect("decode"),
            request
        );
        for field in [
            "actor",
            "account",
            "form",
            "source",
            "revision",
            "validated",
        ] {
            let mut value = serde_json::to_value(&request).expect("value");
            value
                .get_mut("request")
                .expect("request")
                .as_object_mut()
                .expect("object")
                .insert(field.into(), serde_json::json!(true));
            assert!(
                serde_json::from_value::<super::InvokeRequest>(value).is_err(),
                "{field}"
            );
        }
        for licence in ["0", "01", "+1", " 1", "18446744073709551616"] {
            assert!(
                super::InvokeRequest::Mind(super::MindRequest::FiledSource {
                    licence: licence.into(),
                    first: 1,
                    last: 1
                })
                .validate()
                .is_err()
            );
        }
        for (first, last) in [(0, 1), (2, 1), (1, 201), (u32::MAX, 0)] {
            assert!(
                super::InvokeRequest::Mind(super::MindRequest::FiledSource {
                    licence: "1".into(),
                    first,
                    last
                })
                .validate()
                .is_err()
            );
        }
        let legacy = super::InvokeRequest::Mind(super::MindRequest::Read {
            name: "Candidate".into(),
            revision: None,
        });
        assert_eq!(
            serde_json::to_string(&legacy).expect("legacy wire"),
            r#"{"service":"mind","request":{"op":"read","name":"Candidate","revision":null}}"#
        );
    }

    use super::*;
    #[test]
    fn preflight_bounds_bytes_depth_and_escaped_delimiters() {
        assert!(validate_invoke_json(&vec![b' '; MAX_INVOKE_BYTES + 1]).is_err());
        assert!(validate_invoke_json(&[b'['; MAX_INVOKE_DEPTH + 1]).is_err());
        assert!(validate_invoke_json(br#"{"source":"[[[\"[[["}"#).is_ok());
    }
    #[test]
    fn closed_shape_refuses_owner_and_validated_receipts() {
        for json in [
            r#"{"service":"mind","request":{"op":"list","actor":7}}"#,
            r#"{"service":"mind","request":{"op":"check","name":"mine","base":{"edit":1,"revision":null},"validated":true}}"#,
        ] {
            assert!(serde_json::from_str::<InvokeRequest>(json).is_err());
        }
    }

    #[test]
    fn lifecycle_remove_is_closed_and_keeps_its_wire_shape()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = InvokeRequest::Mind(MindRequest::Lifecycle {
            name: "mine".into(),
            base: MindEditBase {
                edit: 1,
                revision: None,
            },
            edit: MindLifecycle::Remove {},
        });
        let wire = serde_json::json!({
            "service": "mind",
            "request": {
                "op": "lifecycle", "name": "mine",
                "base": {"edit": 1, "revision": null},
                "edit": {"kind": "remove"}
            }
        });
        if serde_json::to_value(&request)? != wire {
            return Err("lifecycle remove changed its valid wire shape".into());
        }
        if serde_json::from_value::<InvokeRequest>(wire)? != request {
            return Err("lifecycle remove did not round-trip".into());
        }
        for edit in [
            r#"{"kind":"remove","actor":7}"#,
            r#"{"kind":"remove","account":"foreign"}"#,
            r#"{"kind":"remove","validated":true}"#,
            r#"{"kind":"remove","source":"private"}"#,
            r#"{"kind":"remove","new_name":"other"}"#,
        ] {
            if serde_json::from_str::<MindLifecycle>(edit).is_ok() {
                return Err(format!("lifecycle remove accepted an extra field: {edit}").into());
            }
            let nested = format!(
                r#"{{"service":"mind","request":{{"op":"lifecycle","name":"mine","base":{{"edit":1,"revision":null}},"edit":{edit}}}}}"#
            );
            if serde_json::from_str::<InvokeRequest>(&nested).is_ok() {
                return Err(
                    format!("nested lifecycle remove accepted an extra field: {edit}").into(),
                );
            }
        }
        Ok(())
    }
    /// 17g cut C3a (the C3 plan §4.3): the typed market grammar is closed and
    /// bounded. `validate` refuses a `player` that is empty, over 32
    /// characters or holds a control character, a `price` that is empty,
    /// over 16 bytes or holds a byte outside the digits, `.`, `c` and `r`,
    /// and a royalty over 300; a `ListingCode` of anything but 8 digits and
    /// letters a-f, and an unknown field inside `action`, are refused at
    /// decode. The wire shape is pinned.
    #[test]
    fn market_actions_are_closed_and_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let code = || ListingCode::try_from("0a1b2c3d").map_err(|e| e.to_string());
        let market = |action: MarketAction| InvokeRequest::Mind(MindRequest::Market { action });
        let resell = |player: &str, price: &str| {
            market(MarketAction::Resell {
                licence: 7,
                player: player.into(),
                price: price.into(),
            })
        };
        let wire = serde_json::to_string(&market(MarketAction::Pay { code: code()? }))?;
        check_eq!(
            wire,
            r#"{"service":"mind","request":{"op":"market","action":{"act":"pay","code":"0a1b2c3d"}}}"#
        );
        check_eq!(
            serde_json::from_str::<InvokeRequest>(&wire)?,
            market(MarketAction::Pay { code: code()? })
        );
        for (player, ok) in [
            ("", false),
            (&"p".repeat(32)[..], true),
            (&"p".repeat(33)[..], false),
            ("Old\nCy", false),
            ("Old Cy", true),
        ] {
            check_eq!(resell(player, "1").validate().is_ok(), ok, "{player:?}");
            check_eq!(
                market(MarketAction::Charter {
                    filed: 3,
                    player: player.into(),
                    price: "1".into(),
                })
                .validate()
                .is_ok(),
                ok,
                "charter {player:?}"
            );
        }
        for (price, ok) in [
            ("", false),
            ("1", true),
            ("0.21", true),
            ("1000000.00cr", true),
            (&"1".repeat(16)[..], true),
            (&"1".repeat(17)[..], false),
            ("1 cr", false),
            ("-1", false),
        ] {
            check_eq!(resell("Cy", price).validate().is_ok(), ok, "{price:?}");
            check_eq!(
                market(MarketAction::ResellAccept {
                    licence: 7,
                    price: Some(price.into()),
                })
                .validate()
                .is_ok(),
                ok,
                "accept {price:?}"
            );
        }
        check!(
            market(MarketAction::CharterAccept {
                filed: 3,
                price: None
            })
            .validate()
            .is_ok()
        );
        for (royalty, ok) in [
            (None, true),
            (Some(0), true),
            (Some(300), true),
            (Some(301), false),
        ] {
            check_eq!(
                market(MarketAction::Publish {
                    filed: 3,
                    form: MarketListingForm::OpenResale,
                    royalty,
                    transfer: Some(false),
                })
                .validate()
                .is_ok(),
                ok,
                "{royalty:?}"
            );
        }
        for (text, ok) in [
            ("0a1b2c3", false),
            ("0a1b2c3d", true),
            ("0a1b2c3d4", false),
            ("0A1B2C3D", false),
            ("0a1b2c3g", false),
            ("0a1b-c3d", false),
            ("", false),
        ] {
            check_eq!(ListingCode::try_from(text).is_ok(), ok, "{text:?}");
            let raw = format!(
                r#"{{"service":"mind","request":{{"op":"market","action":{{"act":"quote","code":"{text}"}}}}}}"#
            );
            check_eq!(
                serde_json::from_str::<InvokeRequest>(&raw).is_ok(),
                ok,
                "decode {text:?}"
            );
        }
        for extra in ["actor", "account", "token", "payer", "buyer", "digest"] {
            let raw = format!(
                r#"{{"service":"mind","request":{{"op":"market","action":{{"act":"pay","code":"0a1b2c3d","{extra}":1}}}}}}"#
            );
            check!(
                serde_json::from_str::<InvokeRequest>(&raw).is_err(),
                "{extra}"
            );
        }
        check!(
            serde_json::from_str::<InvokeRequest>(
                r#"{"service":"mind","request":{"op":"market","action":{"act":"pay","code":"0a1b2c3d"},"actor":7}}"#
            )
            .is_err()
        );
        check_eq!(
            serde_json::to_string(&MarketAction::Publish {
                filed: 3,
                form: MarketListingForm::OpenResale,
                royalty: None,
                transfer: None,
            })?,
            r#"{"act":"publish","filed":3,"form":"open_resale","royalty":null,"transfer":null}"#
        );
        check_eq!(
            serde_json::to_string(&MindResponse::Staged { line: "x".into() })?,
            r#"{"status":"staged","line":"x"}"#
        );
        check_eq!(
            serde_json::to_string(&MindResponse::Shown {
                lines: vec!["a".into()]
            })?,
            r#"{"status":"shown","lines":["a"]}"#
        );
        Ok(())
    }

    #[test]
    fn attributed_remove_is_name_only_and_never_reports_committed_removal()
    -> Result<(), Box<dyn std::error::Error>> {
        let wire = r#"{"service":"mind","request":{"op":"remove","name":"mine"}}"#;
        let request: InvokeRequest = serde_json::from_str(wire)?;
        request.validate()?;
        check_eq!(serde_json::to_string(&request)?, wire);
        for field in [
            "actor",
            "account",
            "mind",
            "base",
            "edit",
            "revision",
            "source",
            "validated",
        ] {
            let raw = format!(
                r#"{{"service":"mind","request":{{"op":"remove","name":"mine","{field}":1}}}}"#
            );
            check!(serde_json::from_str::<InvokeRequest>(&raw).is_err());
        }
        check!(
            serde_json::from_str::<InvokeRequest>(
                r#"{"service":"mind","request":{"op":"remove","name":"mine","name":"other"}}"#
            )
            .is_err()
        );
        for name in ["", "mine@1", "mine:other", "a\0b"] {
            check!(
                InvokeRequest::Mind(MindRequest::Remove { name: name.into() })
                    .validate()
                    .is_err()
            );
        }
        let reply = MindResponse::RemovalRequested {
            name: "mine".into(),
        };
        check!(reply.is_final()); // Transport completed; mutation has not been claimed.
        check_eq!(
            serde_json::to_string(&reply)?,
            r#"{"status":"removal_requested","name":"mine"}"#
        );
        check_ne!(
            reply,
            MindResponse::Removed {
                name: "mine".into()
            }
        );
        Ok(())
    }
}
