//! Actual certified Origin -> manufacture -> delivery -> fitting pipeline.
//! No owner receipt, certified asset, native hold, unit replacement or readiness
//! bit is seeded. Captured ACK writes are folded after install, as in the factory
//! parent fixture; separate server lost-ACK delivery is not claimed here.
use super::*;
use crate::core_production::refit::{
    self,
    records::{AssetPosition, RefitOrder, SavedComponent},
};
use crate::workorder::WorkOrderId;
use mush_tactics::{CellAddr, CellId, ComponentId, UnitMutationOp, UnitMutationPlan};
struct Fitting {
    f: FactoryRig,
    hull: ObjId,
    incoming: ObjId,
    manufacture: CommitToken,
    class: DieClass,
}
impl Fitting {
    fn new(reverse: bool) -> Result<Self> {
        let f = FactoryRig::new_options(reverse, true, true)?;
        let mut s = Self {
            hull: GOD,
            incoming: GOD,
            manufacture: token(21),
            class: DieClass::I,
            f,
        };
        let buyer = s.f.buyer;
        let factory = s.f.composition.factory.payment.factory;
        s.f.r.w.world_mut().move_to(buyer, factory)?;
        let hull =
            s.f.r
                .w
                .world_mut()
                .create(ObjKind::Thing, "Refit hull", buyer, Some(factory))?;
        s.hull = hull;
        command(
            &mut s.f.r.w,
            &format!("@unit/create #{}=refit_unit_1", hull.0),
        )?;
        command(
            &mut s.f.r.w,
            &format!("@asset/title #{}=#{}", hull.0, buyer.0),
        )?;
        let scope = s.f.composition.factory.payment.integrator.scope;
        command(
            &mut s.f.r.w,
            &format!("@asset/title #{}=org:Importer scope={}", factory.0, scope.0),
        )?;
        let operator = s.f.r.policy.agents[1].actor;
        command(
            &mut s.f.r.w,
            &format!(
                "@org/grant #{}=Importer industry.core.refit scope=#{}",
                operator.0, scope.0
            ),
        )?;
        s.f.r.flush();
        let id = s.f.accepted()?;
        s.incoming = s.complete_manufacture(id, 22)?;
        s.f.commit(7, buyer, 28, FactoryRig::order_parameters(id))?;
        assert_eq!(
            s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
            Some(buyer)
        );
        s.damage_heat(7)?;
        Ok(s)
    }
    fn runtime(&self) -> Result<refit::runtime::Runtime> {
        self.f
            .r
            .runtime
            .refit
            .clone()
            .ok_or_else(|| "refit runtime".into())
    }
    fn action(&self, phase: usize) -> Result<DefinitionRef> {
        self.runtime()?
            .actions
            .get(phase)
            .cloned()
            .ok_or_else(|| "refit phase".into())
    }
    fn prepare_inv(&mut self, phase: usize, inv: &ActionInvocation, n: u8) -> Result<Prepared> {
        let a = self.action(phase)?;
        if phase == 3 {
            let order = serde_json::from_value(inv.parameters()["order"].clone())?;
            return self
                .f
                .prepare_scheduled(&a, self.order(order)?.progress.cursor().id(), inv, n);
        }
        Ok(self
            .f
            .r
            .w
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&a, inv, token(n))?)
    }
    fn stage(&mut self, phase: usize, inv: &ActionInvocation, n: u8, p: Prepared) -> Result {
        let a = self.action(phase)?;
        self.f.stage_action(a, inv, n, p)
    }
    fn commit(
        &mut self,
        phase: usize,
        actor: ObjId,
        n: u8,
        parameters: Value,
    ) -> Result<Vec<ReplayWrite>> {
        let inv = invocation(actor, n, parameters);
        let p = self.prepare_inv(phase, &inv, n)?;
        self.stage(phase, &inv, n, p)?;
        self.f.take_install()
    }
    fn complete_manufacture(&mut self, id: WorkOrderId, n: u8) -> Result<ObjId> {
        self.f
            .commit(3, self.f.buyer, n, FactoryRig::order_parameters(id))?;
        self.f.advance(id, n + 1)?;
        self.f.advance(id, n + 2)?;
        self.f
            .commit(5, self.f.buyer, n + 3, FactoryRig::order_parameters(id))?;
        match self.f.order(id)?.terminal.as_ref().map(|t| t.result()) {
            Some(CoreTerminalResult::Completed { output, .. }) => Ok(output.object),
            _ => Err("actual manufactured output".into()),
        }
    }
    fn unit(&self) -> Result<mush_tactics::TacticalUnit> {
        Ok(mush_tactics::module::unit_of(self.f.r.w.world(), self.hull)
            .ok_or("actual unit")?
            .clone())
    }
    fn damage_heat(&mut self, value: i64) -> Result {
        let before = self.unit()?;
        let p = UnitMutationPlan::new(
            vec![UnitMutationOp::SetCell {
                addr: CellAddr::Unit {
                    cell: CellId::new("heat")?,
                },
                value,
            }],
            Some(before.revision()),
            Some("retained hull heat".into()),
        )?;
        command(
            &mut self.f.r.w,
            &format!(
                "@unit/damage #{}={}",
                self.hull.0,
                serde_json::to_string(&p)?
            ),
        )?;
        self.f.r.flush();
        Ok(())
    }
    fn offer_parameters(&self) -> Value {
        json!({"requester":self.f.buyer,"hull":self.hull,"incoming":self.incoming,"manufacture":self.manufacture,"class":self.class})
    }
    fn accepted(&mut self, n: u8) -> Result<WorkOrderId> {
        self.commit(
            1,
            self.f.r.policy.agents[1].actor,
            n,
            self.offer_parameters(),
        )?;
        self.commit(
            2,
            self.f.buyer,
            n + 1,
            json!({"offer":token(n),"payer":self.f.payer.0.to_string()}),
        )?;
        Ok(crate::workorder::derive_work_order_id(
            RequestUid::from_bytes([n + 1; 16]),
            0,
        )?)
    }
    fn order(&self, id: WorkOrderId) -> Result<RefitOrder> {
        Ok(self
            .f
            .r
            .w
            .engine()
            .action_state()
            .get(&self.runtime()?.families.orders, &id)?
            .ok_or("refit order")?
            .clone())
    }
    fn advance_inv(&mut self, id: WorkOrderId) -> Result<ActionInvocation> {
        let before = self.order(id)?;
        self.f
            .r
            .w
            .world_mut()
            .set_now(before.progress.cursor().next_due().at);
        self.f
            .scheduled_invocation(&self.action(3)?, before.progress.cursor().id())
    }
    fn ready(&mut self, id: WorkOrderId, n: u8) -> Result {
        for c in [n, n + 1] {
            let inv = self.advance_inv(id)?;
            let p = self.prepare_inv(3, &inv, c)?;
            self.stage(3, &inv, c, p)?;
            self.f.take_install()?;
        }
        Ok(())
    }
    fn position(&self, object: ObjId) -> Result<AssetPosition> {
        Ok(self
            .f
            .r
            .w
            .engine()
            .action_state()
            .get(&self.runtime()?.families.assets, &object)?
            .ok_or("asset head")?
            .position
            .clone())
    }
    fn reservation(&self, id: WorkOrderId) -> Result<Option<Value>> {
        let key = format!(
            "action-state/capacity/v1/{}",
            self.order(id)?.accepted.to_hex()
        );
        let rows: Vec<_> = self
            .f
            .r
            .boot
            .get("industry")
            .ok_or("industry")?
            .kv
            .iter()
            .filter(|(k, _)| k.as_str() == key)
            .map(|(_, v)| v.clone())
            .collect();
        if rows.len() > 1 {
            return Err("duplicate exact refit reservation".into());
        }
        Ok(rows.into_iter().next())
    }
    fn reservation_targets(&self, id: WorkOrderId, displaced: bool) -> Result<Value> {
        let row = self.reservation(id)?.ok_or("actual refit reservation")?;
        let targets = row
            .get("targets")
            .and_then(Value::as_array)
            .ok_or("targets")?;
        let mut expected = vec![
            ("industry_refit_orders", 197_632u64),
            ("industry_refit_assets", 4096),
            ("industry_refit_hulls", 4096),
            ("core_component_exchanges", 16_384),
            ("industry_facility_slots", 1024),
            ("industry_facility_book", 197_632),
            ("industry_refit_schedule", 2048),
            ("industry_refit_schedule_order", 2048),
        ];
        if displaced {
            expected.push(("industry_refit_assets", 4096));
        }
        let mut observed = Vec::new();
        for target in targets {
            let key = target.get("key").ok_or("target key")?;
            assert_eq!(key.get("owner").and_then(Value::as_str), Some("industry"));
            let family = key.get("family").and_then(Value::as_str).ok_or("family")?;
            let ceiling = target
                .get("ceiling")
                .and_then(Value::as_u64)
                .ok_or("ceiling")?;
            assert_eq!(
                target.get("mode").and_then(Value::as_str),
                Some(if family == "core_component_exchanges" {
                    "create"
                } else {
                    "update_existing"
                })
            );
            observed.push((family, ceiling));
        }
        observed.sort();
        expected.sort();
        assert_eq!(observed, expected);
        assert_eq!(targets.len(), if displaced { 9 } else { 8 });
        assert_eq!(
            observed.iter().map(|(_, n)| *n).sum::<u64>(),
            if displaced { 429_056 } else { 424_960 }
        );
        Ok(row)
    }
    fn freeze_title(&mut self, object: ObjId) -> Result {
        command(
            &mut self.f.r.w,
            &format!("@delivery-fixture freeze-title {}", object.0),
        )?;
        self.f.r.flush();
        Ok(())
    }
}

#[test]
fn actual_refit_escrow_schedule_and_two_owner_install_preserve_condition_and_cold_history() -> Result
{
    for reverse in [false, true] {
        let mut s = Fitting::new(reverse)?;
        let money = s.f.balance(s.f.payer)?;
        let before = s.unit()?;
        let id = s.accepted(40)?;
        let reservation = s.reservation_targets(id, false)?;
        assert_eq!(
            s.f.balance(s.f.payer)?,
            (money.0, Amount::from_minor(10_000))
        );
        assert_eq!(
            s.position(s.incoming)?,
            AssetPosition::Pending { order: id }
        );
        let inv = invocation(s.f.buyer, 44, FactoryRig::order_parameters(id));
        assert!(s.prepare_inv(4, &inv, 44).is_err());
        s.ready(id, 42)?;
        assert!(
            s.reservation(id)?.as_ref() == Some(&reservation),
            "progress changed terminal reservation"
        );
        let p = s.prepare_inv(4, &inv, 44)?;
        assert_eq!(s.unit()?, before);
        assert_eq!(
            s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
            Some(s.f.buyer)
        );
        s.stage(4, &inv, 44, p)?;
        let writes = s.f.take_install()?;
        assert_eq!(
            writes
                .iter()
                .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
                .count(),
            3
        );
        assert!(
            s.reservation(id)?.is_none(),
            "installed fitting retained capacity"
        );
        let after = s.unit()?;
        let row = s.order(id)?;
        let installation = row.installation.as_ref().ok_or("actual installed proof")?;
        assert_eq!(after.revision(), before.revision() + 1);
        assert_eq!(
            SavedComponent::from_unit(&after, &ComponentId::new("core")?)?
                .installed
                .asset,
            Some(s.incoming)
        );
        let heat = CellAddr::Unit {
            cell: CellId::new("heat")?,
        };
        assert_eq!(
            before.cells().find(|(k, _)| *k == &heat),
            after.cells().find(|(k, _)| *k == &heat)
        );
        assert_eq!(
            s.f.balance(s.f.payer)?,
            (
                money
                    .0
                    .checked_sub(Amount::from_minor(10_000))
                    .ok_or("fee")?,
                Amount::from_minor(0)
            )
        );
        assert_eq!(
            s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
            Some(s.hull)
        );
        assert_eq!(
            s.position(s.incoming)?,
            AssetPosition::Mounted {
                order: id,
                hull: s.hull
            }
        );
        assert_eq!(
            installation.replacement.inserted.installed.asset,
            Some(s.incoming)
        );
        s.f.restart(CoreOriginMode::Active)?;
        assert_eq!(s.order(id)?, row);
        assert_eq!(s.unit()?, after);
    }
    Ok(())
}

#[test]
fn actual_refit_cancel_releases_exact_fee_and_slot_after_recipe_change_and_expiry() -> Result {
    let mut s = Fitting::new(false)?;
    let money = s.f.balance(s.f.payer)?;
    let id = s.accepted(40)?;
    let before = s.order(id)?;
    assert_eq!(
        before.intent.expires_at.at - before.intent.accepted_at.at,
        7 * 86400
    );
    s.f.r.w.world_mut().set_now(before.intent.expires_at.at);
    let inv = invocation(s.f.buyer, 44, FactoryRig::order_parameters(id));
    assert!(s.prepare_inv(4, &inv, 44).is_err());
    s.f.composition.refits.as_mut().ok_or("rules")?.revision += 1;
    s.f.restart(CoreOriginMode::Active)?;
    s.commit(5, s.f.buyer, 45, FactoryRig::order_parameters(id))?;
    assert_eq!(s.f.balance(s.f.payer)?, money);
    assert_eq!(
        s.position(s.incoming)?,
        AssetPosition::Released { order: id }
    );
    assert_eq!(
        s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
        Some(s.f.buyer)
    );
    s.f.restart(CoreOriginMode::Active)?;
    Ok(())
}

#[test]
fn actual_refit_cancel_repeats_requester_lifecycle_before_prepare_stage_and_take() -> Result {
    for phase in 0..3 {
        let mut s = Fitting::new(false)?;
        let id = s.accepted(40)?;
        let inv = invocation(s.f.buyer, 44, FactoryRig::order_parameters(id));
        let mut prepared = if phase > 0 {
            Some(s.prepare_inv(5, &inv, 44)?)
        } else {
            None
        };
        if phase == 2 {
            s.stage(5, &inv, 44, prepared.take().ok_or("prepared")?)?;
        }
        let buyer = s.f.buyer;
        s.f.r
            .w
            .world_mut()
            .get_mut(buyer)
            .ok_or("requester")?
            .flags
            .insert(mush_core::Flags::GOING);
        s.f.r.w.world_mut().take_dirty();
        let revision = s.f.r.w.engine().action_state().revision();
        match phase {
            0 => assert!(s.prepare_inv(5, &inv, 44).is_err()),
            1 => assert!(
                s.stage(5, &inv, 44, prepared.take().ok_or("prepared")?)
                    .is_err()
            ),
            _ => assert!(s.f.r.w.engine_mut().take_staged_action().is_err()),
        }
        assert_eq!(s.f.r.w.engine().action_state().revision(), revision);
        assert!(s.order(id)?.terminal.is_none());
        assert_eq!(s.f.balance(s.f.payer)?.1, Amount::from_minor(10_000));
    }
    Ok(())
}

#[test]
fn actual_refit_install_repeats_title_and_unit_preimages_at_all_custody_boundaries() -> Result {
    for phase in 0..3 {
        for drift in 0..2 {
            let mut s = Fitting::new(false)?;
            let id = s.accepted(40)?;
            s.ready(id, 42)?;
            let inv = invocation(s.f.buyer, 44, FactoryRig::order_parameters(id));
            let mut p = if phase > 0 {
                Some(s.prepare_inv(4, &inv, 44)?)
            } else {
                None
            };
            if phase == 2 {
                s.stage(4, &inv, 44, p.take().ok_or("prepared")?)?;
            }
            if drift == 0 {
                s.freeze_title(s.incoming)?;
            } else {
                s.damage_heat(9)?;
            }
            let revision = s.f.r.w.engine().action_state().revision();
            match phase {
                0 => assert!(s.prepare_inv(4, &inv, 44).is_err()),
                1 => assert!(s.stage(4, &inv, 44, p.take().ok_or("prepared")?).is_err()),
                _ => assert!(s.f.r.w.engine_mut().take_staged_action().is_err()),
            }
            assert_eq!(s.f.r.w.engine().action_state().revision(), revision);
            assert!(s.order(id)?.terminal.is_none());
            assert_eq!(
                s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
                Some(s.f.buyer)
            );
        }
    }
    Ok(())
}

#[test]
fn actual_second_refit_returns_displaced_core_with_retained_condition_and_current_heads() -> Result
{
    let mut s = Fitting::new(false)?;
    let first = s.accepted(40)?;
    s.ready(first, 42)?;
    s.commit(4, s.f.buyer, 44, FactoryRig::order_parameters(first))?;
    let old = s.incoming;
    // The actual native damage command changes only the installed component's
    // scalar state. Its exact displaced image is retained by the next fitting.
    let core = ComponentId::new("core")?;
    let unit = s.unit()?;
    let damage = UnitMutationPlan::new(
        vec![UnitMutationOp::SetComponentCondition {
            component: core.clone(),
            condition: mush_tactics::ConditionId::new("damaged")?,
        }],
        Some(unit.revision()),
        Some("displaced condition".into()),
    )?;
    command(
        &mut s.f.r.w,
        &format!(
            "@unit/damage #{}={}",
            s.hull.0,
            serde_json::to_string(&damage)?
        ),
    )?;
    s.f.r.flush();
    let displaced = SavedComponent::from_unit(&s.unit()?, &core)?;
    let operator = s.f.r.policy.agents[1].actor;
    let certificate = s.f.r.issue(operator, 60, DieClass::I)?;
    s.f.commit(
        1,
        operator,
        61,
        json!({"requester":s.f.buyer,"class":"i","supplier_lot":certificate.lot.to_string()}),
    )?;
    s.f.commit(2, s.f.buyer, 62, s.f.accept_parameters(61))?;
    let manufacture = crate::workorder::derive_work_order_id(RequestUid::from_bytes([62; 16]), 0)?;
    s.incoming = s.complete_manufacture(manufacture, 63)?;
    s.manufacture = token(62);
    s.f.commit(7, s.f.buyer, 67, FactoryRig::order_parameters(manufacture))?;
    let second = s.accepted(70)?;
    s.reservation_targets(second, true)?;
    s.ready(second, 72)?;
    let writes = s.commit(4, s.f.buyer, 74, FactoryRig::order_parameters(second))?;
    assert_eq!(
        writes
            .iter()
            .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
            .count(),
        4
    );
    assert_eq!(
        s.f.r.w.world().get(old).ok_or("displaced")?.location,
        Some(s.f.buyer)
    );
    assert_eq!(
        s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
        Some(s.hull)
    );
    assert_eq!(
        s.order(second)?
            .installation
            .as_ref()
            .ok_or("installed")?
            .replacement
            .displaced,
        displaced
    );
    assert!(
        matches!(s.position(old)?,AssetPosition::Detached{order,from_hull,..}if order==second&&from_hull==s.hull)
    );
    s.f.restart(CoreOriginMode::Active)?;
    assert!(s.order(first)?.installation.is_some());
    assert!(s.order(second)?.installation.is_some());
    assert_eq!(
        SavedComponent::from_unit(&s.unit()?, &core)?
            .installed
            .asset,
        Some(s.incoming)
    );
    Ok(())
}

#[test]
fn actual_refit_cold_recovery_rejects_each_receipt_head_and_independent_index_omission() -> Result {
    let mut s = Fitting::new(false)?;
    let id = s.accepted(40)?;
    s.ready(id, 42)?;
    s.commit(4, s.f.buyer, 44, FactoryRig::order_parameters(id))?;
    let matches = |value: &Value, name: &str| {
        value
            .get("family")
            .and_then(|f| f.get("id"))
            .and_then(Value::as_str)
            == Some(name)
    };
    for family in [
        "industry_refit_orders",
        "industry_refit_assets",
        "industry_refit_hulls",
        "core_component_exchanges",
    ] {
        let mut boot = s.f.r.boot.clone();
        let mut removed = 0;
        for data in boot.values_mut() {
            data.kv.retain(|(_, v)| {
                let yes = matches(v, family);
                removed += usize::from(yes);
                !yes
            });
        }
        assert_eq!(removed, 1, "one exact history target");
        assert!(
            s.f.cold(boot, &s.f.composition, CoreOriginMode::Active, true)
                .is_err(),
            "missing fitting proof accepted"
        );
    }
    let mut boot = s.f.r.boot.clone();
    let data = boot.get_mut("industry").ok_or("industry")?;
    let before = data.kv.len();
    data.kv
        .retain(|(k, _)| k.as_str() != refit::records::INDEX_KEY);
    assert_eq!(before - data.kv.len(), 1);
    assert!(
        s.f.cold(boot, &s.f.composition, CoreOriginMode::Active, true)
            .is_err(),
        "missing independent index accepted"
    );
    let mut boot = s.f.r.boot.clone();
    for data in boot.values_mut() {
        data.kv.retain(|(_, v)| {
            !v.get("family")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str)
                .is_some_and(|n| {
                    n.starts_with("industry_refit_") || n == "core_component_exchanges"
                })
        });
    }
    assert!(
        s.f.cold(boot, &s.f.composition, CoreOriginMode::Active, true)
            .is_err(),
        "all typed fitting evidence lost"
    );
    for drift in 0..3 {
        let mut boot = s.f.r.boot.clone();
        let mut count = 0;
        for data in boot.values_mut() {
            for (_, v) in &mut data.kv {
                if matches(v, "industry_refit_orders") {
                    let row = v.get_mut("value").ok_or("row")?;
                    match drift {
                        0 => {
                            row["installation"]["core_receipt"] =
                                json!(DefinitionDigest::from_bytes([9; 32]))
                        }
                        1 => row["installation"]["replacement"]["from_revision"] = json!(999),
                        _ => row["funding"]["commit"] = json!(token(99)),
                    };
                    count += 1;
                }
            }
        }
        assert_eq!(count, 1);
        assert!(
            s.f.cold(boot, &s.f.composition, CoreOriginMode::Active, true)
                .is_err(),
            "crossed fitting proof accepted"
        );
    }
    let mut disabled = s.f.composition.clone();
    disabled.refits = None;
    assert!(
        s.f.cold(s.f.r.boot.clone(), &disabled, CoreOriginMode::Active, true)
            .is_err(),
        "disabled fitting accepted retained native evidence"
    );
    Ok(())
}

#[test]
fn actual_refit_pending_carry_is_blocked_and_cancel_restores_ordinary_carry() -> Result {
    let mut s = Fitting::new(false)?;
    let id = s.accepted(40)?;
    let revision = s.f.r.w.engine().action_state().revision();
    let disk = s.f.r.boot.clone();
    s.f.r.w.run(s.f.buyer, &format!("drop #{}", s.incoming.0));
    s.f.r.flush();
    assert_eq!(
        s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
        Some(s.f.buyer)
    );
    assert_eq!(s.f.r.w.engine().action_state().revision(), revision);
    assert!(
        factory_boot_image(&s.f.r.boot) == factory_boot_image(&disk),
        "refused ordinary carry published history"
    );
    s.commit(5, s.f.buyer, 44, FactoryRig::order_parameters(id))?;
    s.f.r.w.run(s.f.buyer, &format!("drop #{}", s.incoming.0));
    s.f.r.flush();
    assert_eq!(
        s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
        Some(s.f.composition.factory.payment.factory)
    );
    s.f.restart(CoreOriginMode::Active)?;
    Ok(())
}

#[test]
fn held_refit_rejects_late_object_change_before_any_owner_or_typed_apply() -> Result {
    let mut s = Fitting::new(false)?;
    let id = s.accepted(40)?;
    s.ready(id, 42)?;
    let inv = invocation(s.f.buyer, 44, FactoryRig::order_parameters(id));
    let p = s.prepare_inv(4, &inv, 44)?;
    s.stage(4, &inv, 44, p)?;
    let held =
        s.f.r
            .w
            .engine_mut()
            .take_staged_action()?
            .ok_or("held fitting")?;
    let (_, install) = held.into_parts();
    let revision = s.f.r.w.engine().action_state().revision();
    let before = s.unit()?;
    let money = s.f.balance(s.f.payer)?;
    s.f.r.w.world_mut().get_mut(s.hull).ok_or("hull")?.name = "changed after custody".into();
    s.f.r.w.world_mut().take_dirty();
    assert!(
        s.f.r
            .w
            .engine_mut()
            .install_composed_action(install)
            .is_err()
    );
    assert_eq!(s.f.r.w.engine().action_state().revision(), revision);
    assert_eq!(s.unit()?, before);
    assert_eq!(s.f.balance(s.f.payer)?, money);
    assert!(s.order(id)?.terminal.is_none());
    Ok(())
}

#[test]
fn actual_refit_scheduler_requires_real_ingress_and_current_dependency_readiness() -> Result {
    for phase in 0..3 {
        for owner in ["industry", "economy", "org", "tactics", "sim"] {
            let mut s = Fitting::new(false)?;
            let id = s.accepted(40)?;
            let inv = s.advance_inv(id)?;
            let mut p = if phase > 0 {
                Some(s.prepare_inv(3, &inv, 42)?)
            } else {
                None
            };
            if phase == 2 {
                s.stage(3, &inv, 42, p.take().ok_or("prepared")?)?;
            }
            let disk = s.f.r.boot.clone();
            let revision = s.f.r.w.engine().action_state().revision();
            s.f.r.w.engine().registry.freeze_mechanics_owner(
                &OwnerId::new(owner)?,
                mush_core::mechanics::IncidentId(1),
                mush_core::mechanics::RecoveryReason::IntegrityViolation {
                    code: super::id("refit_freeze_fixture")?,
                },
            )?;
            match phase {
                0 => assert!(s.prepare_inv(3, &inv, 42).is_err()),
                1 => assert!(s.stage(3, &inv, 42, p.take().ok_or("prepared")?).is_err()),
                _ => assert!(s.f.r.w.engine_mut().take_staged_action().is_err()),
            }
            assert_eq!(s.f.r.w.engine().action_state().revision(), revision);
            assert!(
                factory_boot_image(&s.f.r.boot) == factory_boot_image(&disk),
                "frozen dependency published fitting"
            );
        }
    }
    let mut s = Fitting::new(false)?;
    let id = s.accepted(40)?;
    let real = s.advance_inv(id)?;
    let bad = invocation(s.f.buyer, 42, real.parameters().clone());
    assert!(s.prepare_inv(3, &bad, 42).is_err());
    let before = s.order(id)?;
    let revision = s.f.r.w.engine().action_state().revision();
    let action = s.action(3)?;
    assert!(
        s.f.r
            .w
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&action, &real, token(42))
            .is_err()
    );
    assert!(s.order(id)? == before);
    assert_eq!(s.f.r.w.engine().action_state().revision(), revision);
    let p = s.prepare_inv(3, &real, 42)?;
    s.stage(3, &real, 42, p)?;
    s.f.take_install()?;
    Ok(())
}

#[test]
fn actual_refit_offer_and_accept_are_self_bound_and_do_not_allow_forged_payer_or_image() -> Result {
    let mut s = Fitting::new(false)?;
    let operator = s.f.r.policy.agents[1].actor;
    let bad = invocation(s.f.buyer, 39, s.offer_parameters());
    assert!(s.prepare_inv(1, &bad, 39).is_err());
    s.commit(1, operator, 40, s.offer_parameters())?;
    for actor in [GOD, operator] {
        let inv = invocation(
            actor,
            41,
            json!({"offer":token(40),"payer":s.f.payer.0.to_string()}),
        );
        assert!(s.prepare_inv(2, &inv, 41).is_err());
    }
    for selector in [
        json!({"offer":token(40),"payer":s.f.payer.0}),
        json!({"offer":token(40),"payer":s.f.payer.0.to_string(),"postimage":{}}),
        json!({"offer":token(40),"payer":null}),
        json!({"offer":token(40),"payer":s.f.composition.factory.payment.integrator.account.0.to_string()}),
    ] {
        let inv = invocation(s.f.buyer, 41, selector);
        assert!(s.prepare_inv(2, &inv, 41).is_err());
    }
    s.commit(
        2,
        s.f.buyer,
        41,
        json!({"offer":token(40),"payer":s.f.payer.0.to_string()}),
    )?;
    let inv = invocation(
        s.f.buyer,
        42,
        json!({"offer":token(40),"payer":s.f.payer.0.to_string()}),
    );
    assert!(s.prepare_inv(2, &inv, 42).is_err());
    Ok(())
}

#[test]
fn actual_refit_capacity_survives_progress_and_cold_boot_and_rejects_corrupt_reserved_targets()
-> Result {
    let mut s = Fitting::new(false)?;
    let id = s.accepted(40)?;
    let exact = s.reservation_targets(id, false)?;
    let key = format!(
        "action-state/capacity/v1/{}",
        s.order(id)?.accepted.to_hex()
    );
    for corruption in 0..4 {
        let mut boot = s.f.r.boot.clone();
        let data = boot.get_mut("industry").ok_or("industry")?;
        let before = data.kv.iter().filter(|(k, _)| k.as_str() == key).count();
        assert_eq!(before, 1);
        if corruption == 0 {
            data.kv.retain(|(k, _)| k.as_str() != key);
        } else {
            let row = &mut data
                .kv
                .iter_mut()
                .find(|(k, _)| k.as_str() == key)
                .ok_or("reservation")?
                .1;
            match corruption {
                1 => {
                    row.get_mut("targets")
                        .and_then(Value::as_array_mut)
                        .ok_or("targets")?
                        .pop();
                }
                2 => {
                    row["protocol_fingerprint"] = json!(DefinitionDigest::from_bytes([9; 32]));
                }
                _ => {
                    let first = row
                        .get_mut("targets")
                        .and_then(Value::as_array_mut)
                        .and_then(|v| v.first_mut())
                        .ok_or("first target")?;
                    first["ceiling"] = json!(
                        first
                            .get("ceiling")
                            .and_then(Value::as_u64)
                            .ok_or("ceiling")?
                            + 1
                    );
                }
            }
        }
        assert!(
            s.f.cold(boot, &s.f.composition, CoreOriginMode::Active, true)
                .is_err(),
            "invalid fixed completion capacity accepted"
        );
    }
    s.f.restart(CoreOriginMode::Active)?;
    assert!(
        s.reservation(id)?.as_ref() == Some(&exact),
        "cold reservation changed"
    );
    let inv = s.advance_inv(id)?;
    let p = s.prepare_inv(3, &inv, 42)?;
    s.stage(3, &inv, 42, p)?;
    s.f.take_install()?;
    assert!(
        s.reservation(id)?.as_ref() == Some(&exact),
        "advance reservation changed"
    );
    s.commit(5, s.f.buyer, 44, FactoryRig::order_parameters(id))?;
    assert!(
        s.reservation(id)?.is_none(),
        "cancelled fitting retained capacity"
    );
    s.f.restart(CoreOriginMode::Active)?;
    Ok(())
}

#[test]
fn complete_refit_future_record_bound_fits_every_independent_maximum_inside_real_descriptor()
-> Result {
    let mut s = Fitting::new(false)?;
    let id = s.accepted(40)?;
    let mut value = serde_json::to_value(s.order(id)?)?;
    // Canonical strings upper-bound complete nested JSON values. They are only
    // sizing inputs, never decoded, admitted, installed, or asserted as evidence.
    for (field, cap) in [
        ("intent", 34816usize),
        ("funding", 16384),
        ("progress", 32768),
        ("terminal", 16384),
        ("installation", 69632),
    ] {
        let full = Value::String("x".repeat(cap - 2));
        assert_eq!(
            mush_core::canonical_json_bytes_with_limit(&full, cap)?.len(),
            cap
        );
        value[field] = full;
    }
    value["sequence"] = json!(refit::records::MAX_ORDERS);
    mush_core::canonical_json_bytes_with_limit(&value, refit::records::ORDER_BYTES)?;
    let runtime = s.runtime()?;
    let descriptor = runtime.families.orders.descriptor();
    let envelope = json!({"family":{"owner":"industry","id":"industry_refit_orders"},
        "key":format!("{:016x}",u64::MAX),"schema":{"id":descriptor.value_schema(),"version":descriptor.schema_version()},"value":value});
    let bytes =
        mush_core::canonical_json_bytes_with_limit(&envelope, descriptor.max_record_bytes())?;
    assert!(bytes.len() <= 197_632);
    assert_eq!(descriptor.max_record_bytes(), 197_632);
    Ok(())
}

#[test]
fn actual_class_three_refit_requires_current_personal_entitlement_at_each_install_boundary()
-> Result {
    for boundary in 0..3 {
        let mut s = Fitting::new(false)?;
        let licence = s.f.licence()?;
        let faucet =
            s.f.r
                .w
                .world()
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot()
                .accounts()
                .find(|a| {
                    matches!(&a.owner, PrincipalId::System(n) if n.as_str() == "core.fixture")
                        && a.instrument == s.f.composition.factory.payment.instrument
                })
                .ok_or("cash issuer")?
                .id;
        command(
            &mut s.f.r.w,
            &format!("@econ/mint {}={} 2000000", faucet.0, s.f.payer.0),
        )?;
        s.f.r.flush();
        let operator = s.f.r.policy.agents[1].actor;
        let cert = s.f.r.issue(operator, 60, DieClass::III)?;
        s.f.commit(
            1,
            operator,
            61,
            json!({"requester":s.f.buyer,"class":DieClass::III,"supplier_lot":cert.lot.to_string()}),
        )?;
        s.f.commit(2, s.f.buyer, 62, s.f.accept_parameters(61))?;
        let order = crate::workorder::derive_work_order_id(RequestUid::from_bytes([62; 16]), 0)?;
        s.incoming = s.complete_manufacture(order, 63)?;
        s.manufacture = token(62);
        s.class = DieClass::III;
        s.f.commit(7, s.f.buyer, 67, FactoryRig::order_parameters(order))?;
        let fitting = s.accepted(70)?;
        s.ready(fitting, 72)?;
        let inv = invocation(s.f.buyer, 74, FactoryRig::order_parameters(fitting));
        let mut prepared = if boundary > 0 {
            Some(s.prepare_inv(4, &inv, 74)?)
        } else {
            None
        };
        if boundary == 2 {
            s.stage(4, &inv, 74, prepared.take().ok_or("prepared")?)?;
        }
        command(
            &mut s.f.r.w,
            &format!("@delivery-fixture freeze-account {}", licence.0),
        )?;
        s.f.r.flush();
        match boundary {
            0 => assert!(s.prepare_inv(4, &inv, 74).is_err()),
            1 => assert!(
                s.stage(4, &inv, 74, prepared.take().ok_or("prepared")?)
                    .is_err()
            ),
            _ => assert!(s.f.r.w.engine_mut().take_staged_action().is_err()),
        }
        assert!(s.order(fitting)?.terminal.is_none());
        assert_eq!(
            s.f.r.w.world().get(s.incoming).ok_or("incoming")?.location,
            Some(s.f.buyer)
        );
        // A fresh valid preparation may use the same immutable accepted work.
        command(
            &mut s.f.r.w,
            &format!("@delivery-fixture unfreeze-account {}", licence.0),
        )?;
        s.f.r.flush();
        s.commit(4, s.f.buyer, 75, FactoryRig::order_parameters(fitting))?;
        assert_eq!(s.f.balance(licence)?.0, Amount::from_minor(1));
        assert_eq!(
            s.unit()?.schema(),
            &s.runtime()?.config.payment.fitted_units[2]
        );
        s.f.restart(CoreOriginMode::Active)?;
    }
    Ok(())
}

#[test]
fn actual_accepted_refit_finishes_after_operator_departure_without_new_consent() -> Result {
    let mut s = Fitting::new(false)?;
    let id = s.accepted(40)?;
    let accepted = s.order(id)?;
    let operator = accepted.intent.offer.operator;
    s.f.r
        .w
        .world_mut()
        .get_mut(operator)
        .ok_or("operator")?
        .flags
        .insert(mush_core::Flags::GOING);
    s.f.r.w.world_mut().take_dirty();
    s.ready(id, 42)?;
    s.commit(4, s.f.buyer, 44, FactoryRig::order_parameters(id))?;
    let completed = s.order(id)?;
    assert_eq!(completed.intent, accepted.intent);
    assert_eq!(completed.funding, accepted.funding);
    assert!(completed.installation.is_some());
    s.f.restart(CoreOriginMode::Active)?;
    Ok(())
}

#[test]
fn real_refit_roster_validates_all_six_exact_actions_and_outcome() -> Result {
    for reverse in [false, true] {
        let f = FactoryRig::new_options(reverse, true, true)?;
        let runtime = f.r.runtime.refit.as_ref().ok_or("actual refit runtime")?;
        let definitions = f.r.w.engine().registry.definitions().ok_or("definitions")?;
        for (name, reference) in refit::ACTIONS.iter().zip(&runtime.actions) {
            let expected = runtime.config.action_descriptor(name)?;
            let actual = definitions.get_exact(reference)?;
            assert_eq!(reference, &runtime.config.action(name)?);
            assert_eq!(actual.payload(), expected.payload());
            let mut references = expected.references().to_vec();
            references.sort();
            assert_eq!(actual.references(), references.as_slice());
            assert_eq!(actual.is_current(), expected.is_current());
        }
        let expected = refit::runtime::outcome_descriptor()?;
        let actual = definitions.get_exact(&runtime.outcome)?;
        assert_eq!(actual.payload(), expected.payload());
        assert_eq!(actual.references(), expected.references());
    }
    let absent = FactoryRig::new_options(false, true, false)?;
    let definitions = absent
        .r
        .w
        .engine()
        .registry
        .definitions()
        .ok_or("definitions")?;
    assert!(
        !definitions
            .iter()
            .any(|d| d.reference().key().owner.as_str() == "industry"
                && (refit::ACTIONS.contains(&d.reference().key().id.as_str())
                    || d.reference().key().id.as_str() == "core_refit_changed"))
    );
    Ok(())
}
