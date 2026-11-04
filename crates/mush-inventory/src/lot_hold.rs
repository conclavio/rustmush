//! **Indivisible goods and the lot hold** (doc 39 cut IN.2, Q-39-27): the two seams a part's
//! identity needs from the quantity authority.
//!
//! - **Indivisible goods.** A good the composition names indivisible (tacsim's `class: part`)
//!   is one identified lot from the moment it exists to the moment it is fitted or scrapped:
//!   [`InventoryFamilies::split`], [`InventoryFamilies::merge`] and
//!   [`InventoryFamilies::dissolve`] refuse it with [`InventoryError::Indivisible`], and so do
//!   the verbs that reach them. The class is the game's; this crate asks the composition
//!   through [`IndivisibleGoods`] and has no edge to it (the [`crate::PoolCapacity`] shape).
//! - **The lot hold.** One exact lot held for one holder — a market listing selling that lot,
//!   or a bay order fitting it — so no other path moves, splits, sells or consumes it before
//!   the holder does. The holder alone releases it, relocates it ([`InventoryFamilies::relocate_held`],
//!   the lot-transfer sale: the same id goes to the buyer) or consumes it
//!   ([`InventoryFamilies::consume_held`], a fit). The family is keyed by the lot, so a lot has
//!   at most one holder, and every ordinary lot verb refuses a held lot
//!   ([`InventoryError::ExactInput`], the custody refusal the bay part and core input
//!   protocols already give). Doc 39 §4.11 names `BayPartCustody` for this: that family is the
//!   kernel's bay-refit protocol's, writable only by industry's three refit actions, so the
//!   game's own custody is this hold.

use mush_core::{ActionTxn, CanonicalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::pool::{InventoryError, InventoryFamilies};
use crate::record::{EndpointRef, Lot, LotId};

/// The longest holder text.
pub const MAX_HOLDER_BYTES: usize = 96;

/// **Which goods are indivisible**: a composition seam, answered from the game's goods table.
pub trait IndivisibleGoods: std::fmt::Debug + Send + Sync {
    /// Whether `commodity` is one identified lot a unit, never pooled, split or merged.
    fn indivisible(&self, commodity: &CanonicalId) -> bool;
}

/// **One held lot**: the lot, where it was held, and who holds it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LotHold {
    format: u16,
    lot: LotId,
    endpoint: EndpointRef,
    holder: SmolStr,
}

impl LotHold {
    /// The held lot.
    #[must_use]
    pub const fn lot(&self) -> LotId {
        self.lot
    }
    /// Where it was held.
    #[must_use]
    pub const fn endpoint(&self) -> EndpointRef {
        self.endpoint
    }
    /// Who holds it.
    #[must_use]
    pub fn holder(&self) -> &str {
        &self.holder
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.holder.is_empty()
            || self.holder.len() > MAX_HOLDER_BYTES
            || !self.holder.is_ascii()
        {
            return Err("a lot hold names its holder in 1..=96 ASCII bytes".to_owned());
        }
        Ok(())
    }
}

impl InventoryFamilies {
    /// Whether the composition names `commodity` indivisible.
    #[must_use]
    pub fn is_indivisible(&self, commodity: &CanonicalId) -> bool {
        self.indivisible
            .as_ref()
            .is_some_and(|goods| goods.indivisible(commodity))
    }

    /// Refuse a lot of an indivisible good.
    pub(crate) fn require_divisible(&self, lot: &Lot) -> Result<(), InventoryError> {
        if self.is_indivisible(lot.commodity()) {
            Err(InventoryError::Indivisible)
        } else {
            Ok(())
        }
    }

    /// The hold on `lot`, if one stands.
    ///
    /// # Errors
    /// [`InventoryError::State`] for an unreadable family, or a hold stored under another key.
    pub fn lot_hold(
        &self,
        txn: &mut ActionTxn<'_>,
        lot: LotId,
    ) -> Result<Option<LotHold>, InventoryError> {
        let hold = txn.get(&self.lot_holds, &lot)?.cloned();
        if hold.as_ref().is_some_and(|h| h.lot != lot) {
            return Err(InventoryError::State);
        }
        Ok(hold)
    }

    /// Refuse a lot any holder holds (every ordinary lot verb asks).
    pub(crate) fn require_no_hold(
        &self,
        txn: &mut ActionTxn<'_>,
        lot: LotId,
    ) -> Result<(), InventoryError> {
        if self.lot_hold(txn, lot)?.is_some() {
            Err(InventoryError::ExactInput)
        } else {
            Ok(())
        }
    }

    /// **Hold** `lot`, at `endpoint`, for `holder`.
    ///
    /// # Errors
    /// [`InventoryError::NoLot`] for a lot that is not at `endpoint`;
    /// [`InventoryError::ExactInput`] for a lot already held, by anyone;
    /// [`InventoryError::Identity`] for a holder text out of bounds. Nothing is written on an
    /// error.
    pub fn hold_lot(
        &self,
        txn: &mut ActionTxn<'_>,
        endpoint: EndpointRef,
        lot: LotId,
        holder: &str,
    ) -> Result<Lot, InventoryError> {
        let record = txn
            .get(&self.lots, &lot)?
            .cloned()
            .filter(|l| l.endpoint() == endpoint)
            .ok_or(InventoryError::NoLot)?;
        self.require_unheld_lot(txn, lot)?;
        let hold = LotHold {
            format: 1,
            lot,
            endpoint,
            holder: SmolStr::new(holder),
        };
        hold.validate().map_err(|_| InventoryError::Identity)?;
        txn.insert(&self.lot_holds, &lot, hold)?;
        Ok(record)
    }

    /// The hold on `lot`, when `holder` holds it.
    fn held_by(
        &self,
        txn: &mut ActionTxn<'_>,
        lot: LotId,
        holder: &str,
    ) -> Result<LotHold, InventoryError> {
        self.lot_hold(txn, lot)?
            .filter(|h| h.holder == holder)
            .ok_or(InventoryError::NoLot)
    }

    /// **Release** `holder`'s hold on `lot`, leaving the lot where it lies.
    ///
    /// # Errors
    /// [`InventoryError::NoLot`] when `holder` holds no such lot.
    pub fn release_lot(
        &self,
        txn: &mut ActionTxn<'_>,
        lot: LotId,
        holder: &str,
    ) -> Result<(), InventoryError> {
        self.held_by(txn, lot, holder)?;
        txn.remove(&self.lot_holds, &lot)?;
        Ok(())
    }

    /// **Hand the held lot over** (the lot-transfer sale): `holder`'s hold ends and the lot
    /// itself — the same id, grade and lineage — moves from where it was held to `to`.
    ///
    /// # Errors
    /// [`InventoryError::NoLot`] when `holder` holds no such lot or it has left `from`;
    /// as [`InventoryFamilies::relocate`] otherwise. Nothing is written on an error.
    pub fn relocate_held(
        &self,
        txn: &mut ActionTxn<'_>,
        lot: LotId,
        from: EndpointRef,
        to: EndpointRef,
        holder: &str,
    ) -> Result<Lot, InventoryError> {
        let hold = self.held_by(txn, lot, holder)?;
        if hold.endpoint != from {
            return Err(InventoryError::NoLot);
        }
        txn.remove(&self.lot_holds, &lot)?;
        self.relocate(txn, lot, from, to)
    }

    /// **Consume the held lot** (a fit): `holder`'s hold ends and the lot is consumed, as
    /// [`InventoryFamilies::consume_product`].
    ///
    /// # Errors
    /// [`InventoryError::NoLot`] when `holder` holds no such lot or it changed.
    pub fn consume_held(
        &self,
        txn: &mut ActionTxn<'_>,
        expected: &Lot,
        holder: &str,
    ) -> Result<(), InventoryError> {
        let hold = self.held_by(txn, expected.id(), holder)?;
        if hold.endpoint != expected.endpoint() {
            return Err(InventoryError::NoLot);
        }
        txn.remove(&self.lot_holds, &expected.id())?;
        self.consume_product(txn, expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Grade, InventoryHandleCell, InventoryModule, Lineage};
    use mush_core::{GOD, GameModule, ObjKind};
    use mush_economy::{EconomyModule, EconomyStore, ExternalRef, VaultId};
    use mush_testkit::TestWorld;
    use std::sync::Arc;
    type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

    /// Every good under `test.part.` is indivisible.
    #[derive(Debug)]
    struct Parts;
    impl IndivisibleGoods for Parts {
        fn indivisible(&self, commodity: &CanonicalId) -> bool {
            commodity.as_str().starts_with("test.part.")
        }
    }

    fn vault(w: &mut TestWorld, name: &str) -> Result<EndpointRef> {
        let object = w.world_mut().create(ObjKind::Thing, name, GOD, None)?;
        w.run(
            GOD,
            &format!("@vault/create {name}=container:#{}", object.0),
        );
        Ok(EndpointRef(
            w.world()
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot()
                .vaults()
                .find(|v| v.name.as_str() == name)
                .ok_or("the vault")?
                .id,
        ))
    }

    /// One record, read through an inventory-owned transaction.
    fn read<K, V>(
        w: &mut TestWorld,
        handle: &mush_core::StateFamilyHandle<K, V>,
        key: &K,
    ) -> Option<V>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let mut txn = w.engine_mut().action_transaction(handle).ok()?;
        txn.get(handle, key).ok().flatten().cloned()
    }

    /// One inventory-owned bundle, prepared and installed; its refusal as text.
    fn bundle(
        w: &mut TestWorld,
        f: &InventoryFamilies,
        act: impl FnOnce(&mut ActionTxn<'_>) -> std::result::Result<(), InventoryError>,
    ) -> std::result::Result<(), String> {
        let mut txn = w
            .engine_mut()
            .action_transaction(&f.lots)
            .map_err(|e| e.to_string())?;
        act(&mut txn).map_err(|e| e.to_string())?;
        let (_, install) = txn.prepare().map_err(|e| e.to_string())?.into_parts();
        w.engine_mut()
            .install_prepared_action(install)
            .map_err(|e| e.to_string())
    }

    /// **Doc 39 cut IN.2 at the quantity authority**: a part refuses split, merge and dissolve;
    /// a held lot is its holder's alone, handed over whole by `relocate_held`; a commission lays
    /// an industry lot back as one-unit lots of the same quantity.
    #[test]
    fn in2_a_part_is_one_lot_held_and_handed_over_whole() -> Result {
        let cell: InventoryHandleCell = Arc::default();
        let inventory = InventoryModule::new()
            .with_handle_cell(Arc::clone(&cell))
            .with_indivisible_goods(Arc::new(Parts));
        let modules: Vec<Box<dyn GameModule>> =
            vec![Box::new(EconomyModule::new()), Box::new(inventory)];
        let mut w = TestWorld::with_modules(modules);
        w.world_mut().set_now(1000);
        let seller = vault(&mut w, "Seller hold")?;
        let buyer = vault(&mut w, "Buyer hold")?;
        let f = cell.get().ok_or("families")?.clone();
        let part = CanonicalId::new("test.part.sight")?;
        let origin = |id: &str| ExternalRef {
            provider: "fixture".into(),
            kind: "ordinary_origin".into(),
            id: id.to_owned().into(),
            revision: Some(1),
        };
        let (one, two, three) = (LotId(1), LotId(2), LotId(3));
        bundle(&mut w, &f, |txn| {
            f.mint_origins(
                txn,
                seller,
                &[
                    (part.clone(), 1, Grade::new(9)?, one),
                    (part.clone(), 1, Grade::new(9)?, two),
                    (part.clone(), 3, Grade::new(9)?, three),
                ],
                &origin("1"),
            )
            .map(|_| ())
        })?;
        check!(f.is_indivisible(&part));

        // Never split, merged or dissolved.
        let indivisible = InventoryError::Indivisible.to_string();
        let split = bundle(&mut w, &f, |txn| {
            f.split(txn, seller, three, 1, LotId(9)).map(|_| ())
        });
        check_eq!(split, Err(indivisible.clone()));
        let merged = bundle(&mut w, &f, |txn| {
            f.merge(txn, seller, &[one, two], LotId(9)).map(|_| ())
        });
        check_eq!(merged, Err(indivisible.clone()));
        let dissolved = bundle(&mut w, &f, |txn| f.dissolve(txn, seller, one).map(|_| ()));
        check_eq!(dissolved, Err(indivisible));

        // Held for a sale: no other holder takes it, and nothing consumes it.
        bundle(&mut w, &f, |txn| {
            f.hold_lot(txn, seller, one, "listing/4").map(|_| ())
        })?;
        check!(
            bundle(&mut w, &f, |txn| f
                .hold_lot(txn, seller, one, "tacsim.bay/5/1")
                .map(|_| ()))
            .is_err(),
            "one holder at a time"
        );
        let held = read(&mut w, &f.lots, &one).ok_or("the lot")?;
        check!(
            bundle(&mut w, &f, |txn| f.consume_product(txn, &held)).is_err(),
            "a held lot is consumed by its holder alone"
        );
        check!(
            bundle(&mut w, &f, |txn| f
                .relocate_held(txn, one, seller, buyer, "listing/5")
                .map(|_| ()))
            .is_err(),
            "another listing cannot hand it over"
        );

        // The sale: the lot itself, the same id, goes to the buyer; the hold ends.
        bundle(&mut w, &f, |txn| {
            f.relocate_held(txn, one, seller, buyer, "listing/4")
                .map(|_| ())
        })?;
        let sold = read(&mut w, &f.lots, &one).ok_or("the sold lot")?;
        check_eq!(sold.endpoint(), buyer);
        check_eq!(sold.quantity(), 1);
        check!(read(&mut w, &f.lot_holds, &one).is_none());

        // A fit: held by the order, consumed by it.
        bundle(&mut w, &f, |txn| {
            f.hold_lot(txn, buyer, one, "tacsim.bay/5/1").map(|_| ())
        })?;
        bundle(&mut w, &f, |txn| {
            f.consume_held(txn, &sold, "tacsim.bay/5/1")
        })?;
        check!(read(&mut w, &f.lots, &one).is_none());
        check!(read(&mut w, &f.lot_holds, &one).is_none());

        // A commission: the three-unit lot becomes three one-unit lots, nothing more.
        let source = read(&mut w, &f.lots, &three).ok_or("the source")?;
        let short = bundle(&mut w, &f, |txn| {
            f.commission_lots(txn, &source, &[(LotId(10), Grade::new(9)?)])
                .map(|_| ())
        });
        check!(short.is_err(), "a commission lays back the whole quantity");
        let nine = Grade::new(9)?;
        let rows = [10, 11, 12].map(|n| (LotId(n), nine));
        bundle(&mut w, &f, |txn| {
            f.commission_lots(txn, &source, &rows).map(|_| ())
        })?;
        check!(read(&mut w, &f.lots, &three).is_none());
        for (id, _) in rows {
            let lot = read(&mut w, &f.lots, &id).ok_or("a commissioned lot")?;
            check_eq!(lot.quantity(), 1);
            check_eq!(lot.lineage(), &Lineage::Split { parent: three });
        }
        Ok(())
    }

    #[test]
    fn in2_a_hold_names_its_holder_within_bounds() {
        let hold = |holder: &str| LotHold {
            format: 1,
            lot: LotId(7),
            endpoint: EndpointRef(VaultId(3)),
            holder: SmolStr::new(holder),
        };
        assert!(hold("market.listing/9").validate().is_ok());
        assert!(hold("").validate().is_err());
        assert!(hold(&"x".repeat(MAX_HOLDER_BYTES + 1)).validate().is_err());
        let wire = serde_json::to_value(hold("tacsim.bay/5/2")).expect("encodes");
        let back: LotHold = serde_json::from_value(wire).expect("decodes");
        assert_eq!(back.holder(), "tacsim.bay/5/2");
    }
}
