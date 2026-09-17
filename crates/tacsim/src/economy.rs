//! The one game's economy wiring (16b cut R-b): what `[tacsim.economy]` turns on
//! at the composition root, and the settlement currency the game registers.
//!
//! Doc 11 §2: *the corridor settles in the credit (cr), cleared through Luna.*
//! The instrument is registered create-only at the sheet module's boot, exactly
//! as the WoD ruleset registered `wod.usd` before it (create-only since the E2-C18
//! look-back fold); the ten commodity families are cut C's and arrive as content.

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
    credit
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
