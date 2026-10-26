//! Actual owner output affinity; no paired commercial publication is activated.
use super::*;

#[test]
fn offer_and_close_outputs_bind_exact_material_and_actual_preparation() -> TestResult {
    for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
        let mut rig = Rig::new()?;
        let mut crossed = rig.offer.clone();
        crossed.terms.price = Amount::from_minor(101);
        let expected = rig.offer.digest()?;
        check_ne!(crossed.digest()?, expected);
        let created_at = rig.world.now();
        let (offered, _) = rig.publish()?;
        check_eq!(offered.offer_digest(), expected);
        check_eq!(offered.commit(), token(3));
        check_eq!(offered.commit(), offered.operation());
        check_eq!(offered.action(), &rig.draft);
        check_eq!(offered.posted_at_utc(), created_at);
        check!(offered.closed().is_none());
        let later = if reason == UnfundedCloseReason::Expired {
            rig.offer.terms.expires_at.at
        } else {
            created_at + 1
        };
        rig.world.set_now(later);
        let (closed, patch, _) = rig.close(reason)?;
        check_eq!(closed.operation(), offered.operation());
        check_eq!(closed.contract(), offered.contract());
        check_eq!(closed.consent_hash(), offered.consent_hash());
        check_eq!(closed.offer_digest(), expected);
        check_eq!(closed.commit(), token(4));
        check_ne!(closed.commit(), closed.operation());
        check_eq!(closed.action(), &rig.close);
        check_eq!(closed.posted_at_utc(), rig.world.now());
        check_eq!(closed.at_ic(), &rig.world.world_ic_now());
        check_eq!(closed.closed(), Some(reason));
        patch.recheck_in(&rig.state, &rig.context(true, token(4)))?;
        patch.bind(&mut rig.state)?.apply();
        let actual = rig
            .state
            .renewal_offers
            .get(token(3))
            .ok_or("actual closed row")?;
        let closure = actual.closure.as_ref().ok_or("actual closure")?;
        check_eq!(closure.commit, closed.commit());
        check_eq!(&closure.action, closed.action());
        check_eq!(closure.at_utc, closed.posted_at_utc());
        check_eq!(actual.offer.digest()?, closed.offer_digest());
    }
    Ok(())
}
