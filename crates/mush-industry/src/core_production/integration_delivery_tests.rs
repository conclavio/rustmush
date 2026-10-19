//! Actual factory payment, native completion, current receiving eligibility and
//! Core delivery. The controls below use real Economy APIs to model independent
//! instrument/lifecycle changes; they do not mint factory or delivery evidence.
use super::*;
use factory::delivery::{self, Record};
use mush_economy::{
    AccountStatus, InstrumentDef, InstrumentKind, InstrumentStatus, NegativePolicy,
};
const LICENCE: &str = "tacsim.licence.core3_end_user";

pub(super) fn register_controls(reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "@delivery-fixture".into(),
            shape: ArgShape::Raw,
            ..CommandSpec::default()
        },
        Arc::new(|ctx: &mut Ctx<'_>| {
            if ctx.actor != GOD {
                return Err(CmdError::msg("fixture operator"));
            }
            let fields = ctx
                .args
                .raw()
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            match fields.as_slice() {
                [op] if op == "licence" => mush_economy::register_instrument(
                    ctx,
                    InstrumentDef {
                        id: InstrumentId::new(LICENCE)
                            .ok_or_else(|| CmdError::msg("instrument"))?,
                        ruleset: "tacsim".into(),
                        code: "licence.core3_end_user".into(),
                        kind: InstrumentKind::Entitlement,
                        scale: 0,
                        transferable: false,
                        negative_policy: NegativePolicy::SystemOnly,
                        status: InstrumentStatus::Active,
                        display: None,
                        revision: 1,
                    },
                )
                .map_err(|e| CmdError::msg(e.to_string())),
                [op, id] if op == "freeze-title" || op == "unfreeze-title" => {
                    let object = ObjId(id.parse().map_err(|_| CmdError::msg("object"))?);
                    if !mush_economy::set_asset_frozen(
                        ctx,
                        object,
                        op == "freeze-title",
                        Some("fixture lifecycle".into()),
                    ) {
                        return Err(CmdError::msg("actual title"));
                    }
                    Ok(())
                }
                [op, id] if op == "freeze-account" || op == "unfreeze-account" => {
                    let account = AccountId(id.parse().map_err(|_| CmdError::msg("account"))?);
                    let status = if op == "freeze-account" {
                        AccountStatus::Frozen
                    } else {
                        AccountStatus::Active
                    };
                    if !mush_economy::set_account_status(ctx, account, status) {
                        return Err(CmdError::msg("actual account"));
                    }
                    Ok(())
                }
                _ => Err(CmdError::msg("fixture operation")),
            }
        }),
    )
}
impl FactoryRig {
    fn complete(&mut self, order: crate::workorder::WorkOrderId) -> Result<ObjId> {
        self.commit(3, self.buyer, 22, Self::order_parameters(order))?;
        self.advance(order, 24)?;
        self.advance(order, 25)?;
        self.commit(5, self.buyer, 26, Self::order_parameters(order))?;
        let row = self.order(order)?;
        let Some(CoreTerminalResult::Completed { output, .. }) =
            row.terminal.as_ref().map(|t| t.result())
        else {
            return Err("actual completed output".into());
        };
        Ok(output.object)
    }
    fn receipt(&self, id: crate::workorder::WorkOrderId) -> Result<Option<Record>> {
        let runtime = self.runtime()?;
        let d = runtime.delivery.as_ref().ok_or("delivery runtime")?;
        Ok(self
            .r
            .w
            .engine()
            .action_state()
            .get(&d.records, &id)?
            .cloned())
    }
    fn control(&mut self, line: &str) -> Result {
        command(&mut self.r.w, line)?;
        self.r.flush();
        Ok(())
    }
    pub(super) fn licence(&mut self) -> Result<AccountId> {
        self.control("@delivery-fixture licence")?;
        self.control(&format!("@account/open #{}={LICENCE}", self.buyer.0))?;
        self.control(&format!("@account/system core.licence.fixture={LICENCE}"))?;
        let s = self
            .r
            .w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?
            .snapshot();
        let account = s
            .accounts()
            .find(|a| {
                a.owner == PrincipalId::Object(self.buyer) && a.instrument.as_str() == LICENCE
            })
            .ok_or("personal licence account")?
            .id;
        let faucet = s
            .accounts()
            .find(|a| {
                matches!(&a.owner, PrincipalId::System(n) if n.as_str() == "core.licence.fixture")
                    && a.instrument.as_str() == LICENCE
            })
            .ok_or("licence issuer account")?
            .id;
        self.control(&format!("@econ/mint {}={} 1", faucet.0, account.0))?;
        assert_eq!(self.balance(account)?.0, Amount::from_minor(1));
        Ok(account)
    }
}
fn family(value: &Value, id: &str) -> bool {
    value
        .get("family")
        .and_then(|f| f.get("id"))
        .and_then(Value::as_str)
        == Some(id)
}

#[test]
fn actual_paid_delivery_is_inert_until_install_and_cold_proves_both_receipts() -> Result {
    for reverse in [false, true] {
        let mut f = FactoryRig::new_delivery(reverse, true)?;
        let id = f.accepted()?;
        let object = f.complete(id)?;
        let before_order = f.order(id)?;
        let before_money = f.balance(f.payer)?;
        let title =
            f.r.w
                .world()
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot()
                .asset(object)
                .ok_or("title")?
                .clone();
        let source = f.composition.factory.payment.output_container;
        let before_revision = f.r.w.engine().action_state().revision();
        let inv = invocation(f.buyer, 30, FactoryRig::order_parameters(id));
        let p = f.prepare_inv(7, &inv, 30)?;
        assert_eq!(
            f.r.w.world().get(object).ok_or("object")?.location,
            Some(source)
        );
        assert!(f.receipt(id)?.is_none());
        f.stage_inv(7, &inv, 30, p)?;
        assert_eq!(f.r.w.engine().action_state().revision(), before_revision);
        assert_eq!(
            f.r.w.world().get(object).ok_or("object")?.location,
            Some(source)
        );
        let writes = f.take_install()?;
        assert_eq!(
            writes
                .iter()
                .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
                .count(),
            3
        );
        for name in ["core_output_deliveries", delivery::FAMILY] {
            assert_eq!(writes.iter().filter(|w| matches!(w, ReplayWrite::ModuleJsonSet { value, .. } if family(value, name))).count(), 1);
        }
        assert_eq!(
            f.r.w.world().get(object).ok_or("object")?.location,
            Some(f.buyer)
        );
        assert_eq!(f.order(id)?, before_order);
        assert_eq!(f.balance(f.payer)?, before_money);
        assert_eq!(
            f.r.w
                .world()
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot()
                .asset(object),
            Some(&title)
        );
        let row = f.receipt(id)?.ok_or("actual receipt")?;
        assert_eq!(row.operation, token(30));
        assert_eq!(row.sequence, 1);
        row.matches_order(&before_order)?;
        // Capacity proof uses the actual registered envelope, with every
        // variable-width receipt identity at its permitted maximum. It is not
        // inserted or treated as delivery evidence.
        let mut maximum = row.clone();
        maximum.sequence = factory::records::MAX_ORDERS;
        maximum.order = crate::workorder::WorkOrderId(u64::MAX);
        maximum.object = ObjId(u32::MAX);
        maximum.at_utc = i64::MIN;
        maximum.operation = token(255);
        maximum.accepted = token(254);
        maximum.completion = token(253);
        maximum.reservation = token(252);
        maximum.validate()?;
        let mut envelope = writes
            .iter()
            .find_map(|w| match w {
                ReplayWrite::ModuleJsonSet { value, .. } if family(value, delivery::FAMILY) => {
                    Some(value.clone())
                }
                _ => None,
            })
            .ok_or("actual delivery envelope")?;
        envelope["key"] = json!(format!("{:016x}", u64::MAX));
        envelope["value"] = serde_json::to_value(maximum)?;
        mush_core::canonical_json_bytes_with_limit(&envelope, delivery::BYTES)?;
        assert!(
            f.prepare(7, f.buyer, 31, FactoryRig::order_parameters(id))
                .is_err()
        );
        f.restart(CoreOriginMode::Active)?;
        assert_eq!(f.receipt(id)?, Some(row));
        assert_eq!(
            f.r.w.world().get(object).ok_or("object")?.location,
            Some(f.buyer)
        );
    }
    Ok(())
}

#[test]
fn actual_delivery_requires_completed_exact_requester_and_strict_order_selector() -> Result {
    let mut f = FactoryRig::new_delivery(false, true)?;
    let id = f.accepted()?;
    assert!(
        f.prepare(7, f.buyer, 27, FactoryRig::order_parameters(id))
            .is_err()
    );
    let object = f.complete(id)?;
    let before = f.balance(f.payer)?;
    for actor in [GOD, f.r.policy.agents[1].actor] {
        assert!(
            f.prepare(7, actor, 28, FactoryRig::order_parameters(id))
                .is_err()
        );
    }
    for parameters in [
        json!({"order":id.0}),
        json!({"order":id.to_string(),"recipient":f.buyer}),
        json!({"order":id.to_string(),"object":object}),
        json!({"order":null}),
    ] {
        assert!(f.prepare(7, f.buyer, 29, parameters).is_err());
    }
    assert_eq!(f.balance(f.payer)?, before);
    assert!(f.receipt(id)?.is_none());
    let mut cancelled = FactoryRig::new_delivery(false, true)?;
    let id = cancelled.accepted()?;
    cancelled.commit(6, cancelled.buyer, 27, FactoryRig::order_parameters(id))?;
    assert!(
        cancelled
            .prepare(7, cancelled.buyer, 28, FactoryRig::order_parameters(id))
            .is_err()
    );
    cancelled.restart(CoreOriginMode::Active)?;
    Ok(())
}

#[test]
fn completed_delivery_drains_after_supplier_departure_and_recipe_change() -> Result {
    let mut f = FactoryRig::new_delivery(false, true)?;
    let id = f.accepted()?;
    let object = f.complete(id)?;
    let before = f.order(id)?;
    f.r.deny(f.r.policy.agents[1].actor)?;
    f.composition = f.composition.changed_recipe()?;
    f.restart(CoreOriginMode::Active)?;
    assert_eq!(f.order(id)?, before);
    f.commit(7, f.buyer, 30, FactoryRig::order_parameters(id))?;
    assert_eq!(
        f.r.w.world().get(object).ok_or("object")?.location,
        Some(f.buyer)
    );
    f.restart(CoreOriginMode::Active)?;
    Ok(())
}

#[test]
fn actual_current_title_and_receiver_are_rechecked_before_delivery_custody() -> Result {
    for phase in 0..3 {
        let mut f = FactoryRig::new_delivery(false, true)?;
        let id = f.accepted()?;
        let object = f.complete(id)?;
        let inv = invocation(f.buyer, 30, FactoryRig::order_parameters(id));
        let mut prepared = if phase > 0 {
            Some(f.prepare_inv(7, &inv, 30)?)
        } else {
            None
        };
        if phase == 2 {
            f.stage_inv(7, &inv, 30, prepared.take().ok_or("prepared")?)?;
        }
        f.control(&format!("@delivery-fixture freeze-title {}", object.0))?;
        let revision = f.r.w.engine().action_state().revision();
        match phase {
            0 => assert!(f.prepare_inv(7, &inv, 30).is_err()),
            1 => assert!(
                f.stage_inv(7, &inv, 30, prepared.take().ok_or("prepared")?)
                    .is_err()
            ),
            _ => assert!(f.r.w.engine_mut().take_staged_action().is_err()),
        }
        assert_eq!(f.r.w.engine().action_state().revision(), revision);
        assert!(f.receipt(id)?.is_none());
        assert_eq!(
            f.r.w.world().get(object).ok_or("object")?.location,
            Some(f.composition.factory.payment.output_container)
        );
    }
    let mut f = FactoryRig::new_delivery(false, true)?;
    let id = f.accepted()?;
    f.complete(id)?;
    let buyer = f.buyer;
    f.r.w
        .world_mut()
        .get_mut(buyer)
        .ok_or("buyer")?
        .flags
        .insert(mush_core::Flags::GOING);
    f.r.w.world_mut().take_dirty();
    assert!(
        f.prepare(7, buyer, 30, FactoryRig::order_parameters(id))
            .is_err()
    );
    Ok(())
}

#[test]
fn class_three_delivery_requires_actual_current_personal_entitlement() -> Result {
    let mut f = FactoryRig::new_delivery(false, true)?;
    let licence = f.licence()?;
    let faucet =
        f.r.w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?
            .snapshot()
            .accounts()
            .find(|a| {
                matches!(&a.owner, PrincipalId::System(n) if n.as_str() == "core.fixture")
                    && a.instrument == f.composition.factory.payment.instrument
            })
            .ok_or("cash faucet")?
            .id;
    f.control(&format!("@econ/mint {}={} 2000000", faucet.0, f.payer.0))?;
    let operator = f.r.policy.agents[1].actor;
    let cert = f.r.issue(operator, 40, DieClass::III)?;
    f.commit(
        1,
        operator,
        41,
        json!({"requester":f.buyer,"class":DieClass::III,"supplier_lot":cert.lot.to_string()}),
    )?;
    f.commit(2, f.buyer, 42, f.accept_parameters(41))?;
    let id = crate::workorder::derive_work_order_id(RequestUid::from_bytes([42; 16]), 0)?;
    let object = f.complete(id)?;
    f.control(&format!("@delivery-fixture freeze-account {}", licence.0))?;
    assert!(
        f.prepare(7, f.buyer, 50, FactoryRig::order_parameters(id))
            .is_err()
    );
    assert!(f.receipt(id)?.is_none());
    f.control(&format!("@delivery-fixture unfreeze-account {}", licence.0))?;
    f.commit(7, f.buyer, 51, FactoryRig::order_parameters(id))?;
    assert_eq!(f.balance(licence)?.0, Amount::from_minor(1));
    assert_eq!(
        f.r.w.world().get(object).ok_or("object")?.location,
        Some(f.buyer)
    );
    f.restart(CoreOriginMode::Active)?;
    Ok(())
}

#[test]
fn held_delivery_cannot_bind_changed_physical_preimage_and_publishes_no_receipt() -> Result {
    let mut f = FactoryRig::new_delivery(false, true)?;
    let id = f.accepted()?;
    let object = f.complete(id)?;
    let inv = invocation(f.buyer, 30, FactoryRig::order_parameters(id));
    let p = f.prepare_inv(7, &inv, 30)?;
    f.stage_inv(7, &inv, 30, p)?;
    let held = f.r.w.engine_mut().take_staged_action()?.ok_or("held")?;
    let (_, install) = held.into_parts();
    let before = f.r.w.engine().action_state().revision();
    f.r.w.world_mut().get_mut(object).ok_or("object")?.name = "changed after custody".into();
    f.r.w.world_mut().take_dirty();
    assert!(f.r.w.engine_mut().install_composed_action(install).is_err());
    assert_eq!(f.r.w.engine().action_state().revision(), before);
    assert!(f.receipt(id)?.is_none());
    assert_eq!(
        f.r.w.world().get(object).ok_or("object")?.location,
        Some(f.composition.factory.payment.output_container)
    );
    Ok(())
}

#[test]
fn delivery_cold_inverse_rejects_each_missing_half_crossed_row_and_owner_index() -> Result {
    let mut f = FactoryRig::new_delivery(false, true)?;
    let id = f.accepted()?;
    f.complete(id)?;
    f.commit(7, f.buyer, 30, FactoryRig::order_parameters(id))?;
    for names in [
        vec!["core_output_deliveries"],
        vec![delivery::FAMILY],
        vec!["core_output_deliveries", delivery::FAMILY],
    ] {
        let mut boot = f.r.boot.clone();
        let mut removed = 0;
        for data in boot.values_mut() {
            data.kv.retain(|(_, v)| {
                let yes = names.iter().any(|name| family(v, name));
                removed += usize::from(yes);
                !yes
            });
        }
        assert_eq!(removed, names.len());
        assert!(
            f.cold(boot, &f.composition, CoreOriginMode::Active, true)
                .is_err()
        );
    }
    for field in ["operation", "core_receipt", "sequence"] {
        let mut boot = f.r.boot.clone();
        let mut changed = 0;
        for data in boot.values_mut() {
            for (_, v) in &mut data.kv {
                if family(v, delivery::FAMILY) {
                    let row = v.get_mut("value").ok_or("typed row")?;
                    row[field] = match field {
                        "operation" => json!(token(99)),
                        "sequence" => json!(2),
                        _ => json!(DefinitionDigest::from_bytes([9; 32])),
                    };
                    changed += 1;
                }
            }
        }
        assert_eq!(changed, 1);
        assert!(
            f.cold(boot, &f.composition, CoreOriginMode::Active, true)
                .is_err()
        );
    }
    let mut boot = f.r.boot.clone();
    let data = boot.get_mut("industry").ok_or("industry")?;
    let before = data.kv.len();
    data.kv
        .retain(|(key, _)| key.as_str() != delivery::INDEX_KEY);
    assert_eq!(before - data.kv.len(), 1);
    assert!(
        f.cold(boot, &f.composition, CoreOriginMode::Active, true)
            .is_err()
    );
    assert!(
        f.cold(
            f.r.boot.clone(),
            &f.composition,
            CoreOriginMode::Active,
            false
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn legacy_payment_bytes_remain_absent_and_uninitialized_upgrade_cannot_promise_delivery() -> Result
{
    let mut f = FactoryRig::new(false)?;
    let old = f.composition.factory.payment.clone();
    let value = serde_json::to_value(&old)?;
    assert!(value.get("delivery").is_none());
    assert_eq!(
        serde_json::from_value::<CorePaymentPolicy>(value.clone())?,
        old
    );
    for delivery in [Value::Null, json!("unknown")] {
        let mut bad = value.clone();
        bad["delivery"] = delivery;
        assert!(serde_json::from_value::<CorePaymentPolicy>(bad).is_err());
    }
    assert!(f.phase_action(7).is_err());
    f.composition.factory.payment.delivery =
        Some(mush_economy::core_integration::CoreDeliveryPromise::PersonalCustodyV1);
    // The old factory remains historically readable, but has never installed
    // the independent delivery census needed before a new paid promise.
    f.restart(CoreOriginMode::Active)?;
    assert!(
        f.prepare(1, f.r.policy.agents[1].actor, 20, f.offer_parameters())
            .is_err()
    );
    Ok(())
}

#[test]
fn actual_delivery_offer_is_own_current_completed_order_only() -> Result {
    let mut f = FactoryRig::new_delivery(false, true)?;
    let id = f.accepted()?;
    let object = f.complete(id)?;
    let action = f.phase_action(7)?;
    let keys = mush_core::OfferKeyRing::new(mush_core::OfferSecret::new(1, [17; 32])?, Vec::new())?;
    let choices = |f: &FactoryRig, actor: ObjId| -> Result<Option<Vec<String>>> {
        let query = mush_core::LegalActionQuery::new(
            mush_core::PrincipalId::Object { id: actor },
            actor,
            mush_core::SubjectKey::Object { id: actor },
            PerceivedRef::SelfActor,
            1,
            Some(f.r.w.world().world_ic_now()),
        )?;
        let sink =
            f.r.w
                .engine()
                .registry
                .action_providers()
                .ok_or("providers")?
                .project_legal_actions(f.r.w.engine(), &query, &keys)?;
        let Some(offer) = sink.projected_for_action(&action) else {
            return Ok(None);
        };
        let fields = offer.parameters().parameters();
        assert_eq!(fields.len(), 1);
        let field = fields.first().ok_or("selector")?;
        assert_eq!(field.id.as_str(), "order");
        let mush_core::ProjectedParameterKind::Choice { choices } = &field.kind else {
            return Err("order choices".into());
        };
        Ok(Some(choices.clone()))
    };
    assert_eq!(choices(&f, f.buyer)?, Some(vec![id.to_string()]));
    assert!(choices(&f, GOD)?.is_none());
    assert!(choices(&f, f.r.policy.agents[1].actor)?.is_none());
    f.control(&format!("@delivery-fixture freeze-title {}", object.0))?;
    assert!(choices(&f, f.buyer)?.is_none());
    f.control(&format!("@delivery-fixture unfreeze-title {}", object.0))?;
    f.commit(7, f.buyer, 30, FactoryRig::order_parameters(id))?;
    assert!(choices(&f, f.buyer)?.is_none());
    Ok(())
}

#[test]
fn native_vault_backing_cannot_be_used_as_ordinary_delivery_source() -> Result {
    let mut f = FactoryRig::new_delivery(false, true)?;
    let id = f.accepted()?;
    let object = f.complete(id)?;
    let source = f.composition.factory.payment.output_container;
    f.control(&format!(
        "@vault/create CompletedCoreVault=container:#{} owner=#{}",
        source.0, GOD.0
    ))?;
    assert!(
        f.r.w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?
            .snapshot()
            .is_vault_backing(source)
    );
    let before = f.balance(f.payer)?;
    assert!(
        f.prepare(7, f.buyer, 30, FactoryRig::order_parameters(id))
            .is_err()
    );
    assert!(f.receipt(id)?.is_none());
    assert_eq!(f.balance(f.payer)?, before);
    assert_eq!(
        f.r.w.world().get(object).ok_or("object")?.location,
        Some(source)
    );
    Ok(())
}
