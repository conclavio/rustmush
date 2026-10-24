//! The one game's economy wiring (16b cut R-b): what `[tacsim.economy]` turns on
//! at the composition root, and the settlement currency the game registers.
//!
//! Doc 11 §2: *the corridor settles in the credit (cr), cleared through Luna.*
//! The instrument is registered create-only at the sheet module's boot, exactly
//! as the WoD ruleset registered `wod.usd` before it (create-only since the E2-C18
//! look-back fold); the ten commodity families are cut C's and arrive as content;
//! the Freeport Registry's prospecting licence (doc 20 §9.1, cut SM.5) is one more
//! create-only row, an entitlement.

use mush_core::{Ctx, ObjId};
use mush_economy::{
    EconomyStore, InstrumentDef, InstrumentDisplay, InstrumentId, InstrumentKind, InstrumentStatus,
    NegativePolicy, SymbolPosition,
};
use smol_str::SmolStr;

/// The settlement currency's instrument id.
pub const CREDIT: &str = "tacsim.credit";
/// Its human code and symbol.
pub const CREDIT_CODE: &str = "cr";
/// Its minor-unit scale (centicredits).
pub const CREDIT_SCALE: u8 = 2;

/// **The Freeport Registry's prospecting licence** (doc 20 §9.1, D-20-12; cut
/// SM.5): an `Entitlement` instrument whose `Active` account in a person's name is
/// the whole of what `mush_industry::extraction_authorized` asks of a licence
/// (`crates/mush-industry/src/licence.rs`). Phase 1 has exactly one, because every
/// site of the lane is the Registry's; the term, the grace and the fee are lane
/// data (`PortDef.licence`), but the id is this constant, because the composition
/// root names it to the industry module before any content loads. The term lives
/// on tacsim's own `tacsim:licence:` record (`crate::port::licence`), never on the
/// account, which has none.
pub const LICENCE_FREEPORT: &str = "tacsim.licence.freeport";
/// The licence instrument's human code.
pub const LICENCE_CODE: &str = "FPL";

/// The licence instrument's id ([`LICENCE_FREEPORT`]); `None` only if the constant
/// stopped being a valid id, which a unit test pins never happens.
#[must_use]
pub fn licence_instrument() -> Option<InstrumentId> {
    InstrumentId::new(LICENCE_FREEPORT)
}

/// `[tacsim.economy]`, mapped by the host (`mush_server::config_ext::tacsim_economy_config`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameEconomyConfig {
    /// Whether the housing title issuer (`system:tacsim.housing`) is registered so
    /// `@property/create` can mint tenancies.
    pub housing: bool,
    /// Whether the market's title issuer (`system:tacsim.market`) is registered and
    /// the market composes.
    pub market: bool,
    /// The settlement currency's instrument id (`tacsim.credit`).
    pub instrument: SmolStr,
    /// Extra rooms that may never become property rooms, beside `[onboarding]`'s two.
    pub reserved_rooms: Vec<ObjId>,
}

impl Default for GameEconomyConfig {
    fn default() -> Self {
        GameEconomyConfig {
            housing: true,
            market: false,
            instrument: SmolStr::new_static(CREDIT),
            reserved_rooms: Vec::new(),
        }
    }
}

/// Register the settlement currency if it is not already there. Create-only: an
/// instrument that exists is left exactly as it is, and a refusal is logged rather
/// than failing the boot, because the economy's own `@instrument/register` is the
/// repair path.
pub fn ensure_instrument(ctx: &mut Ctx<'_>, cfg: &GameEconomyConfig) -> Option<InstrumentId> {
    let id = InstrumentId::new(cfg.instrument.as_str())?;
    create_only(
        ctx,
        InstrumentDef {
            id: id.clone(),
            ruleset: SmolStr::new_static("tacsim"),
            code: SmolStr::new_static(CREDIT_CODE),
            kind: InstrumentKind::Currency,
            scale: CREDIT_SCALE,
            transferable: true,
            // A faucet must be able to run negative and `SystemOnly` is the policy
            // `@instrument/register` gives every currency, so an ordinary character's
            // account still cannot.
            negative_policy: NegativePolicy::SystemOnly,
            status: InstrumentStatus::Active,
            display: Some(InstrumentDisplay {
                symbol: SmolStr::new_static(CREDIT_CODE),
                position: SymbolPosition::Suffix,
            }),
            revision: 1,
        },
    )
}

/// **The settlement currency and the ten commodity families** (16b cut C.2, C4;
/// doc 10 S4A, doc 16 §4).
///
/// The same create-only rule, one row at a time: an instrument that exists is
/// left exactly as it is, and a row that will not register is logged and skipped
/// rather than failing the boot. The commodities come from
/// `crates/tacsim/content/resources.json`, which is the authored list and the
/// only place the ten ids and their codes are written down.
///
/// Returns the currency's id, so the one caller that needs it still has it.
pub fn ensure_instruments(ctx: &mut Ctx<'_>, cfg: &GameEconomyConfig) -> Option<InstrumentId> {
    let credit = ensure_instrument(ctx, cfg);
    match crate::definitions::ResourceTable::load(
        "tacsim/content/resources.json",
        crate::definitions::content::RESOURCES_JSON,
    ) {
        Ok(table) => {
            for def in table.instruments() {
                create_only(ctx, def);
            }
        }
        Err(error) => tracing::error!(
            %error,
            "tacsim.economy: the shipped resource table did not decode; no commodity family \
             was registered"
        ),
    }
    ensure_licence_instrument(ctx);
    credit
}

/// **Register the prospecting licence if it is not already there** (doc 20 §9.1,
/// D-20-12; cut SM.5): an indivisible, untransferable entitlement no account may
/// hold below zero. Create-only like every instrument here; a refusal is logged and
/// the caller goes on (the licence verb then answers that the Registry could not
/// issue one). Called from [`ensure_instruments`] at the sheet module's boot and by
/// the port applier for an entry that authors a `licence` block (the credit's
/// [`ensure_instrument`] precedent: a roster without the sheet module still gets
/// it).
pub fn ensure_licence_instrument(ctx: &mut Ctx<'_>) -> Option<InstrumentId> {
    let id = licence_instrument()?;
    create_only(
        ctx,
        InstrumentDef {
            id,
            ruleset: SmolStr::new_static("tacsim"),
            code: SmolStr::new_static(LICENCE_CODE),
            kind: InstrumentKind::Entitlement,
            scale: 0,
            transferable: false,
            negative_policy: NegativePolicy::Forbid,
            status: InstrumentStatus::Active,
            display: None,
            revision: 1,
        },
    )
}

/// Register one instrument if this world has none by that id.
fn create_only(ctx: &mut Ctx<'_>, def: InstrumentDef) -> Option<InstrumentId> {
    let id = def.id.clone();
    let known = ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| s.snapshot().instrument(&id).is_some());
    if known {
        return Some(id);
    }
    if let Err(error) = mush_economy::register_instrument(ctx, def) {
        tracing::warn!(instrument = %id, %error, "tacsim.economy: an instrument was not registered");
        return None;
    }
    Some(id)
}
