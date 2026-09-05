//! **The Shadowrun catalog layer** (E6c, B9) — the ruleset half of `mush-market`.
//!
//! `mush-market` is game-neutral and holds no content, no dice and no idea what a
//! thing *is*. Those three arrive through its wiring-frozen tables, and this module
//! is Shadowrun's registration into all three:
//!
//! | table | this module's entry | what it does |
//! |---|---|---|
//! | [`CatalogSource`] | [`SrCatalogSource`] | the source-controlled `sr.core` starter pack, plus any pack named in `[shadowrun.economy].catalog_paths` |
//! | [`AvailabilityProvider`] | [`SrAvailability`] | an **engine-resolved, recorded** SR5 test through `mush_dice::sr5::{Sr5Pool, Sr5Opts}` under a `RollCtx` |
//! | [`DeliveryProvider`] | [`SrDelivery`] | a commodity or entitlement line, and the installed-augmentation record |
//!
//! ## The starter pack (§11 q8, answer 10)
//!
//! Source-controlled, **≤ 64 entries**, and deliberately *representative rather than
//! exhaustive*: weapons, armour, drugs, programs, a deck, magical goods, a service
//! and cyberware, proving every shipped representation and both availability paths.
//! It ships **no `Lot` entry** — the lot is `mush-inventory`'s in E7 cut 2 (10b §3.2,
//! Q2D), and `mush-market` refuses a `Lot`-represented listing at creation (OPEN 1).
//!
//! ## The baseline moved (E6b's fence, superseded)
//!
//! E6b priced a fence quote from the source-controlled
//! `[shadowrun.economy.fence_baseline]` table keyed on `CatalogRef.code`, because no
//! catalogue existed. It does now, and `CatalogEntry.baseline` is the reference price
//! (doc 08 §9.4 / D9). The config table is **kept only as an override**, asked first
//! so an operator can still correct one code without editing a pack; a `CatalogRef`
//! with no entry and no override keeps E6b's "cannot be quoted" refusal, and hidden
//! provenance still contributes nothing, not even a discount (D-E6-11).
//!
//! ## Rule 2 (08i §4)
//!
//! Every one of the three is a **deterministic capped policy**. No LLM decides a
//! price, a budget or an availability outcome: the price is the catalogue's, the
//! budgets are the maker's declared ones, and the availability test is the same
//! server RNG and the same `Sr5Pool` scoring every player roll uses — reached through
//! the **non-rendering** seam (`agent.rs`'s shape), never `roll.rs`'s `roll_labeled`,
//! which renders to the actor.

use std::sync::Arc;

use mush_core::{Ctx, ObjId, World};
use mush_dice::RollCtx;
use mush_dice::pool::PoolMechanic;
use mush_dice::sr5::{Sr5Opts, Sr5Pool};
use mush_economy::{Amount, CatalogRef, InstrumentId};
use mush_market::{
    AvailabilityOutcome, AvailabilityProvider, AvailabilityRule, CatalogEntry, CatalogSource,
    DeliveryOrder, DeliveryProvider, MarketStore, Representation,
};
use mush_org::PrincipalId;
use mush_ruleset::SheetStore;
use mush_ruleset::schema::SheetSchema;
use smol_str::SmolStr;

use crate::economy::config::SrEconomyConfig;
use crate::economy::record::InstalledAugmentation;
use crate::economy::store::{SrEconomyStore, put_profile};

/// The starter pack's name.
pub const SR_CORE_PACK: &str = "sr.core";

/// The starter pack, source-controlled and compiled in.
const SR_CORE_JSON: &str = include_str!("sr_core.json");

/// The most bytes one on-disk pack file may hold.
///
/// `@market/catalog load` is staff-only and one-shot, but it does read a file from
/// the world task, so the read is bounded: a pack past this is refused by name rather
/// than parsed. 512 KiB is two orders of magnitude past a 512-entry pack.
pub const MAX_PACK_BYTES: usize = 512 * 1_024;

/// The **content-pack source** (B3, B9): the built-in `sr.core`, then whatever
/// `[shadowrun.economy].catalog_paths` names.
///
/// `catalog_paths` stops being inert here. A path may name a directory (searched for
/// `<pack>.json`) or the file itself; the first that reads wins, and a file that will
/// not parse fails **the pack** with its own name in the message, never the server.
#[derive(Debug)]
pub struct SrCatalogSource {
    paths: Vec<String>,
}

impl SrCatalogSource {
    /// Build the source from the frozen `[shadowrun.economy]` block.
    #[must_use]
    pub fn new(cfg: &SrEconomyConfig) -> SrCatalogSource {
        SrCatalogSource {
            paths: cfg.catalog_paths.clone(),
        }
    }
}

impl CatalogSource for SrCatalogSource {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack == SR_CORE_PACK {
            return Some(parse_pack(SR_CORE_PACK, SR_CORE_JSON, "<built-in sr.core>"));
        }
        for base in &self.paths {
            let direct = std::path::Path::new(base);
            let candidate = if direct.is_dir() {
                direct.join(format!("{pack}.json"))
            } else if direct.file_stem().is_some_and(|s| s == pack) {
                direct.to_path_buf()
            } else {
                continue;
            };
            let name = candidate.display().to_string();
            let Ok(meta) = std::fs::metadata(&candidate) else {
                continue;
            };
            if meta.len() > MAX_PACK_BYTES as u64 {
                return Some(Err(format!(
                    "{name} is larger than this server will read ({MAX_PACK_BYTES} bytes)"
                )));
            }
            return Some(match std::fs::read_to_string(&candidate) {
                Ok(text) => parse_pack(pack, &text, &name),
                Err(e) => Err(format!("{name} could not be read: {e}")),
            });
        }
        None
    }
}

/// Parse one pack's JSON, **naming the file** on every failure (B3).
fn parse_pack(pack: &str, text: &str, name: &str) -> Result<Vec<CatalogEntry>, String> {
    let raw: Vec<serde_json::Value> =
        serde_json::from_str(text).map_err(|e| format!("{name} is not a pack: {e}"))?;
    let mut out = Vec::with_capacity(raw.len());
    for (i, mut value) in raw.into_iter().enumerate() {
        // The pack file never has to repeat its own name; the loader stamps it, which
        // is also what makes "an entry claiming another pack" unrepresentable.
        if let Some(map) = value.as_object_mut() {
            map.insert("pack".to_string(), serde_json::Value::String(pack.into()));
        }
        let entry: CatalogEntry = serde_json::from_value(value)
            .map_err(|e| format!("{name}, entry {}: {e}", i.saturating_add(1)))?;
        out.push(entry);
    }
    Ok(out)
}

/// The **availability engine** (B7, answer 11).
///
/// Stocked legal goods need no test at all — `mush-market` never asks. A scarce or
/// restricted entry declares one, and this resolves it through the same server RNG,
/// the same `Sr5Pool` scoring and the same threshold arithmetic every player roll
/// uses. The outcome is **recorded on the quote** and binding thereafter: a re-read
/// never re-rolls, and a failure refuses without charging, because nothing has been
/// charged.
///
/// It is deliberately the **non-rendering** seam (`agent.rs:18-20`'s shape): nothing
/// here writes to the actor's screen, because a market read must not become a roll
/// announcement.
#[derive(Debug)]
pub struct SrAvailability {
    schema: Arc<SheetSchema>,
}

impl Default for SrAvailability {
    fn default() -> SrAvailability {
        SrAvailability::new()
    }
}

impl SrAvailability {
    /// Build the engine over the SR5 sheet schema.
    #[must_use]
    pub fn new() -> SrAvailability {
        SrAvailability {
            schema: Arc::new(crate::schema::build_schema("shadowrun")),
        }
    }

    /// The buyer's dice pool for `skill`, from their **own sheet** and nothing else.
    ///
    /// Zero when they have no sheet or the skill does not resolve, which rolls no
    /// dice and scores no hits — fail closed, and never an error a buyer could read
    /// another character's sheet from.
    fn pool_for(&self, w: &World, who: ObjId, skill: &str) -> i32 {
        let store = SheetStore::new(Arc::clone(&self.schema));
        let Some(doc) = store.get(w, who) else {
            return 0;
        };
        crate::dice::resolve_sheet_pool(&self.schema, &doc, skill)
            .map(|r| r.total)
            .unwrap_or(0)
            .max(0)
    }
}

impl AvailabilityProvider for SrAvailability {
    fn resolve(
        &self,
        ctx: &mut Ctx<'_>,
        rule: &AvailabilityRule,
        catalog: &CatalogRef,
        buyer: ObjId,
    ) -> Option<AvailabilityOutcome> {
        // Only this ruleset's own catalogue, so a WoD or TacSim entry falls through
        // to whichever provider owns it.
        if catalog.ruleset.as_str() != "sr" {
            return None;
        }
        let AvailabilityRule::Test {
            skill, threshold, ..
        } = rule
        else {
            return None;
        };
        let pool = self.pool_for(ctx.w, buyer, skill.as_str());
        let opts = Sr5Opts {
            threshold: *threshold,
            ..Sr5Opts::default()
        };
        let outcome = {
            let st = ctx.w.module_state_mut::<crate::state::Sr5State>()?;
            let mut rctx = RollCtx::new(st.rng());
            Sr5Pool.roll(pool, &opts, &mut rctx)
        };
        let hits = u8::try_from(outcome.hits).unwrap_or(u8::MAX);
        let note = SmolStr::new(format!(
            "{skill} {pool} dice vs {threshold}: {} hit(s)",
            outcome.hits
        ));
        Some(AvailabilityOutcome {
            passed: outcome.is_success(),
            hits,
            threshold: *threshold,
            at: ctx.w.now(),
            note,
        })
    }
}

/// The **delivery provider** (B6): what a settled purchase actually hands over.
///
/// `Representation::Asset` never reaches here — its title moved as an ordinary E4b2
/// `Deliverable::TitleTransfer` inside the settlement itself. The other three are:
///
/// - **`Instrument`** — a fungible commodity line. The goods instrument is
///   `<ruleset>.<code>`, which staff register with `@instrument/register`; the vendor
///   holds the stock in its own account there, and delivery is one ordinary journal
///   post from it to the buyer's, through `mush_economy::deliver_settled`. The
///   vendor's account is the whole of its stock, so a vendor that has sold out is
///   refused by arithmetic and there is no second bookkeeping of quantity anywhere.
/// - **`Entitlement`** — the same act on an `InstrumentKind::Entitlement`
///   instrument. Non-transferability is the **catalogue's** policy in E6c, not the
///   engine's: 08g §4 records that `transferable: false` is recorded and never
///   enforced, and that it matters when **E7 requisitions** arrive.
/// - **`Installed`** — the `InstalledAugmentation` record on the buyer's
///   `SrProfile`, and **nothing else**. Payment never edits a sheet (D-E6-16): the
///   free-text `augmentations` map, `derive::essence_from_augmentations` and
///   `+karma/spend` are all untouched, and the atomic installation that revalidates
///   Essence, prerequisites, custody and clinic capability is still the later clinics
///   slice. This is the **first production caller** of `SrProfile::install`, which
///   E6b's independent verify fold (M11) recorded as shipping without one.
#[derive(Debug, Default)]
pub struct SrDelivery;

impl DeliveryProvider for SrDelivery {
    fn deliver(&self, ctx: &mut Ctx<'_>, order: &DeliveryOrder) -> Option<Result<(), String>> {
        if order.catalog.ruleset.as_str() != "sr" {
            return None;
        }
        match order.representation {
            Representation::Asset | Representation::Lot => None,
            Representation::Instrument | Representation::Entitlement => {
                Some(deliver_line(ctx, order))
            }
            Representation::Installed => Some(deliver_augmentation(ctx, order)),
        }
    }

    /// **The goods leg, asked before the money moves** (E6c independent verify fold,
    /// M1).
    ///
    /// `deliver_line` needs a registered goods instrument, a seller account in it, a
    /// buyer account in it, and enough stock under `CreditPolicy::forbid` — and all four
    /// are absent by default, because nothing in the market registers an instrument,
    /// opens a goods account or checks stock against `Listing.quantity` (a vendor could
    /// list a hundred units while holding five). Before this, every one of those was
    /// discovered at settlement, where the money had already gone. Asked here, they are
    /// a refusal at `+market/accept` with nothing charged.
    ///
    /// It asks exactly the questions the delivery will ask and posts nothing, so a pass
    /// here is not a promise: a world change between the accept and the settlement is
    /// still reachable, and that residual is what the escalation ladder in
    /// `mush_market::module::deliver` is for.
    fn can_deliver(&self, ctx: &Ctx<'_>, order: &DeliveryOrder) -> Option<Result<(), String>> {
        if order.catalog.ruleset.as_str() != "sr" {
            return None;
        }
        match order.representation {
            Representation::Asset | Representation::Lot => None,
            Representation::Instrument | Representation::Entitlement => {
                Some(can_deliver_line(ctx, order))
            }
            Representation::Installed => Some(can_deliver_augmentation(ctx, order)),
        }
    }
}

/// The four questions `deliver_line` will ask, asked without posting.
fn can_deliver_line(ctx: &Ctx<'_>, order: &DeliveryOrder) -> Result<(), String> {
    let Some(instrument) = goods_instrument(&order.catalog) else {
        return Err("that catalogue code is not a usable instrument id".to_string());
    };
    let Some(from) = mush_market::principal_account(ctx, &instrument, &order.seller) else {
        return Err(format!(
            "the seller holds no {} account to deliver from",
            instrument.as_str()
        ));
    };
    if mush_market::principal_account(ctx, &instrument, &order.buyer).is_none() {
        return Err(format!(
            "the buyer holds no {} account to deliver into",
            instrument.as_str()
        ));
    }
    let wanted = i64::try_from(order.quantity).unwrap_or(i64::MAX);
    let available = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().account(from).cloned())
        .map_or(0, |a| {
            a.posted_balance
                .minor()
                .saturating_sub(a.held_amount.minor())
        });
    if available < wanted {
        return Err(format!(
            "the seller holds {available} of {} and the sale is for {wanted}",
            instrument.as_str()
        ));
    }
    Ok(())
}

/// The one question `deliver_augmentation` will ask, asked without writing.
fn can_deliver_augmentation(ctx: &Ctx<'_>, order: &DeliveryOrder) -> Result<(), String> {
    let PrincipalId::Object(who) = order.buyer else {
        return Err("only a character can have ware fitted".to_string());
    };
    if ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who))
        .is_none()
    {
        return Err("the buyer has no Shadowrun profile to record the ware on".to_string());
    }
    Ok(())
}

/// The goods instrument for one catalogue reference: `<ruleset>.<code>`.
#[must_use]
pub fn goods_instrument(catalog: &CatalogRef) -> Option<InstrumentId> {
    InstrumentId::new(format!("{}.{}", catalog.ruleset, catalog.code))
}

fn deliver_line(ctx: &mut Ctx<'_>, order: &DeliveryOrder) -> Result<(), String> {
    let Some(instrument) = goods_instrument(&order.catalog) else {
        return Err("that catalogue code is not a usable instrument id".to_string());
    };
    let Some(from) = mush_market::principal_account(ctx, &instrument, &order.seller) else {
        return Err(format!(
            "the seller holds no {} account to deliver from",
            instrument.as_str()
        ));
    };
    let Some(to) = mush_market::principal_account(ctx, &instrument, &order.buyer) else {
        return Err(format!(
            "the buyer holds no {} account to deliver into",
            instrument.as_str()
        ));
    };
    let amount = Amount::from_minor(i64::try_from(order.quantity).unwrap_or(i64::MAX));
    mush_economy::deliver_settled(ctx, order.contract, from, to, amount)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn deliver_augmentation(ctx: &mut Ctx<'_>, order: &DeliveryOrder) -> Result<(), String> {
    let PrincipalId::Object(who) = order.buyer else {
        return Err("only a character can have ware fitted".to_string());
    };
    let Some(mut profile) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
    else {
        return Err("the buyer has no Shadowrun profile to record the ware on".to_string());
    };
    let essence = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&order.catalog).map(essence_of))
        .unwrap_or(0);
    let now = ctx.w.now();
    let aug = InstalledAugmentation {
        object: None,
        catalog: order.catalog.code.clone(),
        grade: SmolStr::new_static("standard"),
        essence_cost: essence,
        clinic: SmolStr::new_static("market"),
        acquisition: None,
        installed_at: now,
    };
    if !profile.install(aug) {
        return Err("that character already carries as much ware as the record holds".to_string());
    }
    put_profile(ctx, profile);
    Ok(())
}

/// The declared Essence cost (×100) an entry carries, read from its `essence:<n>`
/// tag. Zero when it declares none — recorded, never enforced: the sheet's own
/// Essence arithmetic is the clinics slice's (D-E6-16).
fn essence_of(e: &CatalogEntry) -> u32 {
    e.tags
        .iter()
        .find_map(|t| t.strip_prefix("essence:"))
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0)
        .min(600)
}

/// The **catalogue baseline** for one code, in minor units — E6b's fence basis, moved
/// (B9).
///
/// The `[shadowrun.economy.fence_baseline]` table is asked **first**, as an override
/// an operator can correct one code with; then the catalogue's own
/// `CatalogEntry.baseline`. A code with neither cannot be quoted, which is exactly
/// E6b's refusal, unchanged.
#[must_use]
pub fn baseline_minor(ctx: &Ctx<'_>, cfg: &SrEconomyConfig, code: &str) -> Option<i64> {
    if let Some(n) = cfg.baseline_minor(code) {
        return Some(n);
    }
    let catalog = CatalogRef {
        ruleset: SmolStr::new_static("sr"),
        code: SmolStr::new(code),
    };
    ctx.w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&catalog).and_then(|e| e.baseline))
        .map(|b| b.minor())
        .filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_starter_pack_parses_and_is_representative() {
        let entries = parse_pack(SR_CORE_PACK, SR_CORE_JSON, "sr_core.json").expect("the pack");
        assert!(
            entries.len() <= 64,
            "§11 q8 caps the starter pack at 64 entries; it has {}",
            entries.len()
        );
        for want in [
            Representation::Asset,
            Representation::Instrument,
            Representation::Entitlement,
            Representation::Installed,
        ] {
            assert!(
                entries.iter().any(|e| e.representation == want),
                "the starter pack proves no {want:?} entry"
            );
        }
        assert!(
            !entries
                .iter()
                .any(|e| e.representation == Representation::Lot),
            "OPEN 1: the starter pack ships no Lot entry"
        );
        assert!(
            entries.iter().any(|e| e.availability.needs_test()),
            "the starter pack proves no tested availability path"
        );
        assert!(
            entries
                .iter()
                .any(|e| matches!(e.availability, AvailabilityRule::None)),
            "the starter pack proves no stocked availability path"
        );
        assert!(
            entries.iter().all(|e| e.pack.as_str() == SR_CORE_PACK),
            "the loader stamps the pack name"
        );
    }

    #[test]
    fn a_bad_entry_fails_the_pack_and_names_the_file() {
        let bad = r#"[{"catalog":{"ruleset":"sr","code":"x"},"representation":"asset"}]"#;
        let err = parse_pack("sr.core", bad, "packs/sr.core.json").expect_err("refused");
        assert!(err.contains("packs/sr.core.json"), "{err}");
        assert!(err.contains("entry 1"), "{err}");
    }
}
