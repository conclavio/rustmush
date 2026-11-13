#![cfg(test)]

//! Doc 46 CT.0: codec pins of the pending-charter records (`closed!`) and the
//! protocol's registered definitions (`tests/ct0/wire_charter_pending.snap`).
//!
//! `PendingRecord` (the hand-written strict pair) is not pinned: its fixture needs
//! a validated native `CharterPendingHistory`, which only the Economy owner writes.
use super::super::protocol;
use super::super::records::*;
use super::buyer_offers::offer;
use crate::commerce::charter_transfer::records::CharterOwnershipRef;
use crate::commerce::families::CommerceFamilies;
use crate::commerce::records::{OwnerTerm, RevisionPolicy};
use crate::commerce::tests::ct0_rig::{self as rig, TestResult};
use crate::commerce::{CharterId, LicenceId, LineId, TermGeneration, TermWindow, WorldIcSeconds};
use mush_core::{CommitToken, DefinitionDigest, ObjId};
use mush_economy::managed_charter_transfer::CharterPendingDecisionKindV1;
use mush_testkit::snapshot::Snapshot;

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}

fn families(s: &mut Snapshot, f: &CommerceFamilies) {
    s.section("families");
    rig::family(s, &f.charter_pending_offers);
    rig::family(s, &f.charter_pending_buyer_offers);
    rig::family(s, &f.charter_pending);
    rig::family(s, &f.charter_pending_heads);
}

#[test]
fn ct0_charter_pending_wire() -> TestResult {
    let (_engine, f) = rig::families()?;
    let mut s = Snapshot::new();
    families(&mut s, &f);
    s.line("MAX_PENDING_HISTORY", &MAX_PENDING_HISTORY.to_string());
    s.line("MAX_PENDING_ROW_BYTES", &MAX_PENDING_ROW_BYTES.to_string());
    s.line("MAX_BUYER_OFFERS", &MAX_BUYER_OFFERS.to_string());

    s.section("protocol");
    for (n, d) in protocol::definitions()?.iter().enumerate() {
        rig::descriptor(&mut s, &format!("definition {n}"), crate::NS, d)?;
    }
    s.wire("offer action", &protocol::offer_action()?)?;
    s.wire("fund action", &protocol::fund_action()?)?;
    s.wire("settle action", &protocol::settle_action()?)?;
    s.wire("refund action", &protocol::refund_action()?)?;

    s.section("owner term");
    let term = OwnerTerm {
        format: 1,
        licence: LicenceId::new(7)?,
        generation: TermGeneration::new(1)?,
        window: TermWindow::new(
            WorldIcSeconds(900),
            WorldIcSeconds(900),
            WorldIcSeconds(1100),
            WorldIcSeconds(1200),
        )?,
        revision: RevisionPolicy::Follow {
            line: LineId::new(2)?,
        },
        operation: token(1),
        parent: None,
        attestation: None,
    };
    s.wire("owner term", &term)?;
    let captured = PendingOwnerTerm::capture(&term)?;
    s.wire("owner term digest", &captured.digest)?;
    captured.matches(&term)?;

    s.section("offers");
    let wide = super::wide_offer()?;
    let plain = offer(10, ObjId(8), 100, 1000)?;
    for (label, row) in [("wide", &wide), ("plain", &plain)] {
        rig::document(
            &mut s,
            &format!("{label} offer"),
            &f.charter_pending_offers,
            row,
        )?;
        s.wire(&format!("{label} offer digest"), &row.digest()?)?;
        s.wire(&format!("{label} offer native terms"), &row.native()?)?;
    }
    let index = PendingBuyerOffers {
        format: 1,
        buyer: ObjId(8),
        charter: CharterId::new(100)?,
        offers: [token(10), token(11)].into_iter().collect(),
    };
    rig::document(
        &mut s,
        "buyer offers",
        &f.charter_pending_buyer_offers,
        &index,
    )?;

    s.section("heads");
    let current = PendingRef {
        sequence: 1,
        operation: token(20),
        digest: DefinitionDigest::from_bytes([3; 32]),
    };
    let transfer = CharterOwnershipRef {
        generation: 2,
        operation: token(21),
        digest: DefinitionDigest::from_bytes([4; 32]),
    };
    let states = [
        PendingState::Funded {},
        PendingState::Settled {
            terminal: token(21),
            transfer,
        },
        PendingState::Refunded {
            terminal: token(22),
            reason: CharterPendingDecisionKindV1::ClockChanged,
        },
        PendingState::Refunded {
            terminal: token(22),
            reason: CharterPendingDecisionKindV1::RecipientUnavailable,
        },
        PendingState::Refunded {
            terminal: token(22),
            reason: CharterPendingDecisionKindV1::RegistryRevoked {
                sanction: token(22),
                licence: 7,
                generation: 1,
                term_digest: DefinitionDigest::from_bytes([5; 32]),
            },
        },
    ];
    let mut heads = Vec::new();
    for (n, state) in states.into_iter().enumerate() {
        let head = PendingHead {
            format: 1,
            charter: CharterId::new(100)?,
            current: current.clone(),
            state,
        };
        rig::document(
            &mut s,
            &format!("head {n}"),
            &f.charter_pending_heads,
            &head,
        )?;
        heads.push(head);
    }

    let other_action =
        serde_json::to_value(crate::commerce::charter_transfer::protocol::offer_action()?)?;
    s.codec_shape("PendingOwnerTerm", &captured)?;
    s.codec("PendingOffer (wide)", &wide, |j| j["action"] = other_action)?;
    s.codec("PendingOffer (plain)", &plain, |j| j["format"] = 2.into())?;
    s.codec("PendingBuyerOffers", &index, |j| {
        j["offers"] = serde_json::json!([])
    })?;
    s.codec(
        "ClaimObservation",
        &ClaimObservation {
            generation: 3,
            next_transition: 4,
            open_owned: 1,
        },
        |j| j["next_transition"] = 0.into(),
    )?;
    for (n, head) in heads.iter().enumerate() {
        s.codec(&format!("PendingHead ({n})"), head, |j| {
            j["format"] = 2.into()
        })?;
    }
    s.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/wire_charter_pending.snap",
    )?;
    Ok(())
}
