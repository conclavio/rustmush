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
        work_point: None,
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

#[test]
fn high68_rejects_nonexistent_and_non_player_ids_and_clears_empty_invitations() -> TestResult {
    struct EnabledTacsim;
    impl mush_core::GameModule for EnabledTacsim {
        fn name(&self) -> &'static str {
            crate::NS
        }
        fn register(&mut self, reg: &mut mush_core::Registrar<'_>) -> Result<(), ModError> {
            let mut store = crate::TacsimStore::default();
            store.cfg.enabled = true;
            reg.state(store);
            Ok(())
        }
    }
    let mut world = Engine::with_modules(
        World::new_minimal(),
        mush_core::Registry::new(),
        vec![Box::new(EnabledTacsim)],
    )?
    .world;
    world.set_now(1_000);
    let organizer = world.create(
        mush_core::ObjKind::Player,
        "Organizer",
        mush_core::GOD,
        None,
    )?;
    let invitee = world.create(mush_core::ObjKind::Player, "Invitee", mush_core::GOD, None)?;
    let thing = world.create(mush_core::ObjKind::Thing, "Crate", organizer, None)?;
    let going = world.create(mush_core::ObjKind::Player, "Departed", mush_core::GOD, None)?;
    world
        .get_mut(going)
        .ok_or("going player")?
        .flags
        .insert(Flags::GOING);
    let book = PlanningBook::new(organizer)?;
    for bogus in [ObjId(999_999), thing, going] {
        let intent = PendingIntent {
            at: 1_000,
            before_revision: book.revision,
            operation: Operation {
                organizer,
                change: PlanningOperation::CreateDraft { plan: plan(bogus)? },
            },
        };
        check!(
            evaluate(&world, &book, organizer, &intent).is_err(),
            "bogus player {bogus:?} must be rejected in CreateDraft"
        );
        let preset_intent = PendingIntent {
            at: 1_000,
            before_revision: book.revision,
            operation: Operation {
                organizer,
                change: PlanningOperation::CreatePreset {
                    name: "Bogus".into(),
                    plan: plan(bogus)?,
                },
            },
        };
        check!(
            evaluate(&world, &book, organizer, &preset_intent).is_err(),
            "bogus player {bogus:?} must be rejected in CreatePreset"
        );
    }
    let valid_intent = PendingIntent {
        at: 1_000,
        before_revision: book.revision,
        operation: Operation {
            organizer,
            change: PlanningOperation::CreateDraft {
                plan: plan(invitee)?,
            },
        },
    };
    let (created, _) = evaluate(&world, &book, organizer, &valid_intent)?;
    let edit_bogus = PendingIntent {
        at: 1_000,
        before_revision: created.book.revision,
        operation: Operation {
            organizer,
            change: PlanningOperation::EditDraft {
                id: created.affected,
                expected_revision: 1,
                plan: plan(ObjId(888_888))?,
            },
        },
    };
    check!(
        evaluate(&world, &created.book, organizer, &edit_bogus).is_err(),
        "bogus player must be rejected in EditDraft"
    );
    let created_indices = invitation_changes(&book, &created.book, created.affected, |_| Ok(None))?;
    let initial = created_indices
        .first()
        .ok_or("initial invitation")?
        .after
        .clone();
    let discarded = created.book.apply(
        organizer,
        &PlanningOperation::DiscardDraft {
            id: created.affected,
            expected_revision: 1,
        },
    )?;
    let removed = invitation_changes(&created.book, &discarded.book, discarded.affected, |_| {
        Ok(Some(initial.clone()))
    })?;
    let cleared = removed.first().ok_or("cleared invitation")?;
    check!(cleared.before.is_some());
    check!(cleared.after.drafts.is_empty());
    Ok(())
}
