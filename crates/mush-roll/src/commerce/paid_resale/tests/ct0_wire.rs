//! Doc 46 CT.0: codec pins of the paid-resale records (`checked!`), the
//! protocol's definitions and every digest over them
//! (`tests/ct0/wire_paid_resale.snap`).
//!
//! `PaidResaleRecord` (the hand-written strict pair) is not pinned: its fixture
//! needs a validated native `PaidResaleHistoryV1`, which only the Economy owner writes.
use super::super::records::HolderTerm;
use super::*;
use crate::commerce::paid::{PaidForm, PaidTermRecord, PaidTerms};
use crate::commerce::rating::RatingClass;
use crate::commerce::records::RevisionPolicy;
use crate::commerce::tests::ct0_rig as rig;
use crate::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermWindow, WorldIcSeconds,
};
use mush_core::ClockRef;
use mush_economy::managed_paid_resale::{PaidHolderRefV1, PaidResaleDecisionKindV1};
use mush_testkit::snapshot::Snapshot;

fn binding(birth: &PaidBirthRefV1) -> Result<PaidResaleBinding> {
    let revision: RevisionIdentity = serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":1,"mind":1,"number":1},"digest":hash(1)},
        "source":hash(2)}))?;
    let term = PaidTermRecord::original(
        LicenceId::new(1)?,
        TermGeneration::new(1)?,
        TermWindow::new(
            WorldIcSeconds(10),
            WorldIcSeconds(10),
            WorldIcSeconds(110),
            WorldIcSeconds(130),
        )?,
        PaidTerms::original_personal_form(RatingClass::I, PaidForm::OpenResale {}, true)?,
        RevisionPolicy::Pin {
            filed: FiledRevisionId::new(3)?,
            revision,
        },
        token(1),
    )?;
    let row = PaidResaleBinding {
        licence: LicenceId::new(1)?,
        charter: CharterId::new(2)?,
        line: LineId::new(3)?,
        birth: birth.clone(),
        ownership: PaidHolderRefV1 {
            generation: 1,
            operation: birth.operation,
            digest: birth.digest,
        },
        term,
        term_clock: ClockRef::WorldIc { generation: 1 },
        economics_digest: hash(8),
        royalty_chain_digest: hash(9),
    };
    row.validate()?;
    Ok(row)
}

#[test]
fn ct0_paid_resale_wire() -> Result {
    let (_engine, f) = rig::families()?;
    let mut s = Snapshot::new();
    s.section("families");
    rig::family(&mut s, &f.paid_resales);
    rig::family(&mut s, &f.paid_resale_heads);
    rig::family(&mut s, &f.paid_holders);

    s.section("protocol");
    for (n, d) in protocol::definitions()?.iter().enumerate() {
        rig::descriptor(&mut s, &format!("definition {n}"), "market", d)?;
    }
    for (n, a) in protocol::actions()?.iter().enumerate() {
        s.wire(&format!("action {n}"), a)?;
    }

    s.section("holder");
    let (birth, holder) = holder()?;
    s.wire("birth", &birth)?;
    rig::document(&mut s, "holder", &f.paid_holders, &holder)?;
    s.wire("holder digest", &holder.digest()?)?;
    s.wire("holder ownership", &holder.ownership()?)?;

    s.section("binding");
    let binding = binding(&birth)?;
    s.wire("binding", &binding)?;
    s.wire(
        "binding term digest",
        &PaidResaleBinding::term_digest(binding.term())?,
    )?;
    s.wire("binding native basis", &binding.native_basis()?)?;

    s.section("heads");
    let current = PaidResaleRef {
        sequence: 1,
        operation: token(20),
        digest: hash(5),
    };
    let mut states = vec![
        PaidResaleState::Funded {},
        PaidResaleState::Settled {
            terminal: token(21),
            ownership: holder.ownership()?,
        },
        PaidResaleState::Refunded {
            terminal: token(22),
            reason: PaidResaleDecisionKindV1::RegistryRevoked {
                sanction: token(22),
                licence: 1,
                generation: 1,
                term_digest: hash(6),
            },
        },
    ];
    for reason in [
        PaidResaleDecisionKindV1::ClockChanged {},
        PaidResaleDecisionKindV1::RecipientUnavailable {},
        PaidResaleDecisionKindV1::MonetaryBindingChanged {},
    ] {
        states.push(PaidResaleState::Refunded {
            terminal: token(22),
            reason,
        });
    }
    let mut heads = Vec::new();
    for (n, state) in states.into_iter().enumerate() {
        let head = PaidResaleHead {
            format: 1,
            licence: LicenceId::new(1)?,
            current: current.clone(),
            state,
        };
        rig::document(&mut s, &format!("head {n}"), &f.paid_resale_heads, &head)?;
        heads.push(head);
    }

    s.codec("PaidResaleBinding", &binding, |j| {
        j["term_clock"] = json!({"kind":"world_ic","generation":0})
    })?;
    s.codec("PaidResaleRef", &current, |j| j["sequence"] = 0.into())?;
    s.codec(
        "ResaleClaimObservation",
        &ResaleClaimObservation {
            generation: 3,
            next_transition: 4,
            open_paid: 1,
        },
        |j| j["next_transition"] = 0.into(),
    )?;
    for (n, head) in heads.iter().enumerate() {
        s.codec(&format!("PaidResaleHead ({n})"), head, |j| {
            j["format"] = 2.into()
        })?;
    }
    s.codec_shape(
        "HolderTerm",
        &HolderTerm {
            generation: TermGeneration::new(1)?,
            operation: token(1),
            digest: hash(3),
        },
    )?;
    s.codec("PaidHolderRecord", &holder, |j| j["format"] = 2.into())?;
    s.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/wire_paid_resale.snap",
    )?;
    Ok(())
}
