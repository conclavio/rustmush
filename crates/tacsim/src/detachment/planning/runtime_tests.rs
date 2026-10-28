#![cfg(test)]

use super::*;
use mush_tactics::detachment::Slot;
use mush_tactics::detachment::planning::{Plan, SlotPlan};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn plan(player: ObjId) -> Result<Plan, Box<dyn std::error::Error>> {
    Ok(Plan {
        carrier: ObjId(10),
        slots: vec![SlotPlan {
            slot: Slot::new(1)?,
            unit: Some(ObjId(11)),
            controller: PlannedController::Player { character: player },
            role: None,
            mind: None,
        }],
        npc_commander: None,
        financial_terms: None,
    })
}

#[test]
fn changed_roster_replaces_invitations_and_discard_removes_them() -> TestResult {
    let before = PlanningBook::new(ObjId(1))?;
    let created = before.apply(
        ObjId(1),
        &PlanningOperation::CreateDraft {
            plan: plan(ObjId(2))?,
        },
    )?;
    let indices = invitation_changes(&before, &created.book, created.affected, |_| Ok(None))?;
    check_eq!(indices.len(), 1);
    let first = indices.first().ok_or("first invitation")?;
    check_eq!(first.actor, ObjId(2));
    let index = first.after.clone();
    check!(index.drafts.contains(&DraftRef {
        organizer: ObjId(1),
        id: created.affected
    }));
    let edited = created.book.apply(
        ObjId(1),
        &PlanningOperation::EditDraft {
            id: created.affected,
            expected_revision: 1,
            plan: plan(ObjId(3))?,
        },
    )?;
    let indices = invitation_changes(&created.book, &edited.book, edited.affected, |actor| {
        Ok((actor == ObjId(2)).then(|| index.clone()))
    })?;
    check_eq!(indices.len(), 2);
    check!(
        indices
            .first()
            .ok_or("removed invitation")?
            .after
            .drafts
            .is_empty()
    );
    let second = indices.get(1).ok_or("added invitation")?;
    check_eq!(second.actor, ObjId(3));
    let added = second.after.clone();
    let discarded = edited.book.apply(
        ObjId(1),
        &PlanningOperation::DiscardDraft {
            id: edited.affected,
            expected_revision: 2,
        },
    )?;
    let removed = invitation_changes(&edited.book, &discarded.book, discarded.affected, |_| {
        Ok(Some(added.clone()))
    })?;
    check_eq!(removed.len(), 1);
    check!(
        removed
            .first()
            .ok_or("discarded invitation")?
            .after
            .drafts
            .is_empty()
    );
    Ok(())
}

#[test]
fn full_invitation_index_refuses_the_whole_proposal() -> TestResult {
    let before = PlanningBook::new(ObjId(1))?;
    let created = before.apply(
        ObjId(1),
        &PlanningOperation::CreateDraft {
            plan: plan(ObjId(2))?,
        },
    )?;
    let full = InvitationIndex {
        revision: 1,
        drafts: (1..=32)
            .map(|id| DraftRef {
                organizer: ObjId(99),
                id: PlanId(id),
            })
            .collect(),
    };
    full.validate()?;
    check!(
        invitation_changes(&before, &created.book, created.affected, |_| Ok(Some(
            full.clone()
        )))
        .is_err()
    );
    check!(before.drafts.is_empty());
    check_eq!(full.drafts.len(), 32);
    Ok(())
}

#[test]
fn decline_releases_inbox_capacity_and_reaccept_or_edit_invites_again() -> TestResult {
    let owner = ObjId(1);
    let player = ObjId(2);
    let before = PlanningBook::new(owner)?;
    let created = before.apply(
        owner,
        &PlanningOperation::CreateDraft {
            plan: plan(player)?,
        },
    )?;
    let created_indices =
        invitation_changes(&before, &created.book, created.affected, |_| Ok(None))?;
    let initial_index = created_indices
        .first()
        .ok_or("initial invitation")?
        .after
        .clone();
    let draft = created
        .book
        .drafts
        .get(&created.affected)
        .ok_or("created draft")?;
    let declaration = |state| PlanningOperation::Declare {
        id: draft.id,
        expected_revision: draft.revision,
        expected_hash: draft.plan_hash.clone(),
        state,
    };
    let declined = created
        .book
        .apply(player, &declaration(ConsentState::Declined))?;
    let removed = invitation_changes(&created.book, &declined.book, declined.affected, |_| {
        Ok(Some(initial_index.clone()))
    })?;
    let empty_index = removed.first().ok_or("declined invitation")?.after.clone();
    check!(empty_index.drafts.is_empty());
    check!(
        declined
            .book
            .drafts
            .get(&draft.id)
            .ok_or("declined draft")?
            .visible_to(player),
        "decline frees the inbox without concealing the exact assigned draft"
    );
    let accepted = declined
        .book
        .apply(player, &declaration(ConsentState::Accepted))?;
    let added = invitation_changes(&declined.book, &accepted.book, accepted.affected, |_| {
        Ok(Some(empty_index.clone()))
    })?;
    check_eq!(
        added.first().ok_or("accepted invitation")?.after.drafts,
        initial_index.drafts
    );
    let mut changed_plan = draft.plan.clone();
    changed_plan.slots.first_mut().ok_or("slot")?.role = Some("support".into());
    let edited = declined.book.apply(
        owner,
        &PlanningOperation::EditDraft {
            id: draft.id,
            expected_revision: draft.revision,
            plan: changed_plan,
        },
    )?;
    let reinvited = invitation_changes(&declined.book, &edited.book, edited.affected, |_| {
        Ok(Some(empty_index.clone()))
    })?;
    check_eq!(
        reinvited.first().ok_or("revised invitation")?.after.drafts,
        initial_index.drafts
    );
    let discarded = declined.book.apply(
        owner,
        &PlanningOperation::DiscardDraft {
            id: draft.id,
            expected_revision: draft.revision,
        },
    )?;
    check!(
        invitation_changes(
            &declined.book,
            &discarded.book,
            discarded.affected,
            |_| Err(OwnerReadError::Integrity)
        )?
        .is_empty(),
        "discarding a declined assignment must not remove the same index twice"
    );
    Ok(())
}

#[test]
fn invitation_codec_refuses_omitted_unknown_and_duplicate_entries() -> TestResult {
    let index = InvitationIndex {
        revision: 1,
        drafts: BTreeSet::from([DraftRef {
            organizer: ObjId(1),
            id: PlanId(1),
        }]),
    };
    let value = serde_json::to_value(&index)?;
    check_eq!(
        serde_json::from_value::<InvitationIndex>(value.clone())?,
        index
    );
    for field in ["schema", "revision", "drafts"] {
        let mut bad = value.clone();
        bad.as_object_mut().ok_or("object")?.remove(field);
        check!(serde_json::from_value::<InvitationIndex>(bad).is_err());
    }
    let mut bad = value.clone();
    let entries = bad
        .get_mut("drafts")
        .and_then(Value::as_array_mut)
        .ok_or("drafts")?;
    entries.push(entries.first().cloned().ok_or("entry")?);
    check!(serde_json::from_value::<InvitationIndex>(bad).is_err());
    let mut bad = value;
    bad.as_object_mut()
        .ok_or("object")?
        .insert("public".into(), json!(true));
    check!(serde_json::from_value::<InvitationIndex>(bad).is_err());
    Ok(())
}
