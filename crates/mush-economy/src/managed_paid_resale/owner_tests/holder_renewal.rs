//! The paid basis in this owner fixture is inert commercial data. The native
//! settlement and its complete cold inventory below are actual E4 evidence.
use super::*;

fn accepted(mode: PaidResaleModeV1, route: PaidBirthRouteV1) -> Result<Fixture> {
    let mut f = Fixture::new()?;
    let mut terms = f.terms()?;
    terms.basis.birth.route = route;
    let action = f.rules.actions[if mode == PaidResaleModeV1::Immediate {
        1
    } else {
        2
    }]
    .clone();
    let prepared = f.prepare(
        &action,
        f.buyer,
        2,
        Input::Accept {
            terms,
            payer: AccountId(3),
            mode,
            claims: PaidResaleClaimsV1 {
                next_transition: 4,
                open_paid: u32::from(mode == PaidResaleModeV1::Pending),
            },
        },
    )?;
    f.publish(action, 2, prepared)?;
    Ok(f)
}
fn holder_basis(f: &Fixture) -> Result<RenewalHolderBasisV1> {
    let h = history(f)?;
    Ok(RenewalHolderBasisV1 {
        acceptance: h.consent().operation,
        ownership: PaidHolderRefV1 {
            generation: h.consent().terms.basis.holder.generation + 1,
            operation: h
                .terminal()
                .map_or(h.consent().operation, |t| t.decision().operation),
            // This isolated native fixture cannot mint the Roll holder digest.
            // The actual paired Roll owner must prove it before admission.
            digest: DefinitionDigest::from_bytes([97; 32]),
        },
    })
}
fn require(f: &Fixture, b: &RenewalHolderBasisV1, route: PaidBirthRouteV1) -> Result {
    b.require_origin(
        &f.state().fold.paid_resale_receipts,
        1,
        f.buyer,
        f.shell,
        1,
        token(1),
        &f.world.world().world_ic_now(),
        f.world.world().now(),
        route,
    )?;
    Ok(())
}

#[test]
fn holder_renewal_origin_requires_actual_settlement_and_exact_buyer_and_generation() -> Result {
    for route in [PaidBirthRouteV1::Entitlement, PaidBirthRouteV1::House] {
        let f = accepted(PaidResaleModeV1::Immediate, route)?;
        let basis = holder_basis(&f)?;
        require(&f, &basis, route)?;
        let cold = f.cold(f.boot.clone())?;
        require(&cold, &basis, route)?;
        let mut bad = basis;
        bad.acceptance = token(99);
        assert!(require(&f, &bad, route).is_err());
        bad = basis;
        bad.ownership.generation += 1;
        assert!(require(&f, &bad, route).is_err());
        bad = basis;
        bad.ownership.operation = token(98);
        assert!(require(&f, &bad, route).is_err());
        assert!(require(&f, &basis, PaidBirthRouteV1::Legacy).is_err());
        let book = &f.state().fold.paid_resale_receipts;
        let at = f.world.world().world_ic_now();
        for (licence, buyer, charter, predecessor, operation) in [
            (2, f.buyer, f.shell, 1, token(1)),
            (1, f.actor, f.shell, 1, token(1)),
            (1, f.buyer, GOD, 1, token(1)),
            (1, f.buyer, f.shell, 1, token(3)),
        ] {
            assert!(
                basis
                    .require_origin(
                        book,
                        licence,
                        buyer,
                        charter,
                        predecessor,
                        operation,
                        &at,
                        f.world.world().now(),
                        route
                    )
                    .is_err()
            );
        }
        let mut earlier = at.clone();
        earlier.at -= 1;
        assert!(
            basis
                .require_origin(
                    book,
                    1,
                    f.buyer,
                    f.shell,
                    1,
                    token(1),
                    &earlier,
                    f.world.world().now(),
                    route
                )
                .is_err()
        );
        assert!(
            basis
                .require_origin(
                    book,
                    1,
                    f.buyer,
                    f.shell,
                    1,
                    token(1),
                    &at,
                    f.world.world().now() - 1,
                    route
                )
                .is_err()
        );
        assert!(
            basis
                .require_origin(
                    &book::PaidResaleBook::default(),
                    1,
                    f.buyer,
                    f.shell,
                    1,
                    token(1),
                    &at,
                    f.world.world().now(),
                    route
                )
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn holder_renewal_origin_distinguishes_actual_pending_refund_and_settlement() -> Result {
    for settle in [false, true] {
        let mut f = accepted(PaidResaleModeV1::Pending, PaidBirthRouteV1::Entitlement)?;
        assert!(require(&f, &holder_basis(&f)?, PaidBirthRouteV1::Entitlement).is_err());
        let reason = if settle {
            PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 }
        } else {
            PaidResaleDecisionKindV1::RecipientUnavailable {}
        };
        if !settle {
            f.world
                .world_mut()
                .get_mut(f.buyer)
                .ok_or("buyer")?
                .flags
                .insert(mush_core::Flags::GOING);
            let _ = f.world.world_mut().take_dirty();
        }
        let action = f.rules.actions[if settle { 3 } else { 4 }].clone();
        let prepared = f.terminal(3, reason)?;
        f.publish(action, 3, prepared)?;
        let basis = holder_basis(&f)?;
        assert_eq!(
            require(&f, &basis, PaidBirthRouteV1::Entitlement).is_ok(),
            settle
        );
        let cold = f.cold(f.boot.clone())?;
        assert_eq!(
            require(&cold, &basis, PaidBirthRouteV1::Entitlement).is_ok(),
            settle
        );
    }
    Ok(())
}
