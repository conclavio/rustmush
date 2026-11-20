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
fn decline_releases_inbox_capacity_and_persists_across_edits() -> TestResult {
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
    let owner = world.create(
        mush_core::ObjKind::Player,
        "Organizer",
        mush_core::GOD,
        None,
    )?;
    let player = world.create(mush_core::ObjKind::Player, "Invitee", mush_core::GOD, None)?;
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
    // M29: editing a declined draft through `evaluate` preserves `ConsentState::Declined` so
    // trivial edits cannot re-insert the draft into the victim's invitation inbox.
    let mut changed_plan = draft.plan.clone();
    changed_plan.slots.first_mut().ok_or("slot")?.role = Some("support".into());
    let edit_intent = PendingIntent {
        at: 1_000,
        before_revision: declined.book.revision,
        operation: Operation {
            organizer: owner,
            change: PlanningOperation::EditDraft {
                id: draft.id,
                expected_revision: draft.revision,
                plan: changed_plan,
            },
        },
    };
    let (edited, _) = evaluate(&world, &declined.book, owner, &edit_intent)?;
    let edited_draft = edited.book.drafts.get(&draft.id).ok_or("edited draft")?;
    check_eq!(
        edited_draft.consents.get(&player).map(|c| c.state),
        Some(ConsentState::Declined)
    );
    check!(
        invitation_changes(&declined.book, &edited.book, edited.affected, |_| {
            Ok(Some(empty_index.clone()))
        })?
        .is_empty(),
        "editing a declined draft must not re-insert into the recipient's inbox"
    );
    // Explicit acceptance on the edited draft still restores the invitation cleanly.
    let reaccepted = edited.book.apply(
        player,
        &PlanningOperation::Declare {
            id: edited_draft.id,
            expected_revision: edited_draft.revision,
            expected_hash: edited_draft.plan_hash.clone(),
            state: ConsentState::Accepted,
        },
    )?;
    let restored = invitation_changes(&edited.book, &reaccepted.book, reaccepted.affected, |_| {
        Ok(Some(empty_index.clone()))
    })?;
    check_eq!(
        restored.first().ok_or("restored invitation")?.after.drafts,
        initial_index.drafts
    );
    // Discarding a declined draft (even after edit) must not hit integrity errors.
    let discarded = edited.book.apply(
        owner,
        &PlanningOperation::DiscardDraft {
            id: edited_draft.id,
            expected_revision: edited_draft.revision,
        },
    )?;
    check!(
        invitation_changes(&edited.book, &discarded.book, discarded.affected, |_| Err(
            OwnerReadError::Integrity
        ))?
        .is_empty(),
        "discarding a declined edited assignment must not remove the same index twice"
    );
    Ok(())
}

#[test]
fn m29_per_organizer_invitation_cap_limits_single_organizer_slots() -> TestResult {
    let org_a = ObjId(10);
    let org_b = ObjId(20);
    let victim = ObjId(30);
    let mut book_a = PlanningBook::new(org_a)?;
    let mut victim_index: Option<InvitationIndex> = None;

    for n in 0..families::MAX_INVITATIONS_PER_ORGANIZER {
        let next = book_a.apply(
            org_a,
            &PlanningOperation::CreateDraft {
                plan: plan(victim)?,
            },
        )?;
        let changes = invitation_changes(&book_a, &next.book, next.affected, |_| {
            Ok(victim_index.clone())
        })?;
        check_eq!(changes.len(), 1);
        victim_index = Some(changes[0].after.clone());
        check_eq!(victim_index.as_ref().map(|i| i.drafts.len()), Some(n + 1));
        book_a = next.book;
    }

    // One more draft from the same organizer to the same victim must hit the per-organizer cap.
    let third = book_a.apply(
        org_a,
        &PlanningOperation::CreateDraft {
            plan: plan(victim)?,
        },
    )?;
    let err = invitation_changes(&book_a, &third.book, third.affected, |_| {
        Ok(victim_index.clone())
    });
    check_eq!(err.err(), Some(OwnerReadError::Limit));

    // A different organizer can still invite the same victim cleanly.
    let book_b = PlanningBook::new(org_b)?;
    let other = book_b.apply(
        org_b,
        &PlanningOperation::CreateDraft {
            plan: plan(victim)?,
        },
    )?;
    let changes_b = invitation_changes(&book_b, &other.book, other.affected, |_| {
        Ok(victim_index.clone())
    })?;
    check_eq!(changes_b.len(), 1);
    check_eq!(
        changes_b[0].after.drafts.len(),
        families::MAX_INVITATIONS_PER_ORGANIZER + 1
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
