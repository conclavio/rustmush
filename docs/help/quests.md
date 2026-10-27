# Quests

Some NPCs have authored follow-up work for people they have met. Conversation
can introduce it, but the quest service checks eligibility, records the work
and confirms completion. You can use the commands with the model disabled.

  +quest [<giver>]                 Read your own progress or available work.
  +quest/offer <giver>             Read the invitation and its acceptance token.
  +quest/accept <giver>=<token>    Accept the exact invitation printed by the service.
  +quest/check <giver>             Record an objective at its required location.
  +quest/finish <giver>            Return to the giver and share completed observations.

Acceptance and finishing require the giver to be present and mutually
perceivable. A local check requires you to be in the objective's actual room.
The service records that room detail's current public text; saying you visited,
remembering an old description, or merely looking at it does not record a check.
Your progress and service answers are private to you.

## A second reading

After bringing Leila Mensah-Kovács home on a completed, paid passage, meet her
again at Ceres's Visiting Berth Gallery. She is the same person you travelled
with. `+quest Leila` checks whether her follow-up is available to you; an arrival
whose fare is still settling is not yet a paid passage.

Leila asks for a fresh observation of the Exchange Hall's sample tap. Read the
offer, accept its exact token, then follow its directions. At the Exchange use
`+quest/check Leila`. Return to her and use `+quest/finish Leila` to share the
recorded observation. This is a small exercise in looking carefully, not a
repair, water-safety test or delivery of an imaginary tool.

Finishing gives Leila one verified work experience and a personal relationship
reward. There is no credit payment. The completed record survives restart and
cannot be repeated for more rewards. Another character cannot claim your passage
or your progress, even if they have the same name.

## Authoring a local-detail quest

A `quest` entry belongs to an ordinary `npc` entry in the same pack. The current
eligibility rule requires that NPC's completed, paid passage with the asker to
the configured destination port. Add the `quest` interface and `+quest` command
reference to its profile. Let the service supply the asker's invitation and
progress; do not put unearned objectives or completion claims into public lore,
persona or fallback text.

```json
{
  "quest.second_reading": {
    "kind": "quest",
    "def": {
      "npc": "npc.visitor",
      "title": "A second reading",
      "introduction": "Would you check the sample tap and bring back what you observe?",
      "destination": "ceres",
      "checkpoints": [
        {
          "id": "sample_tap",
          "title": "Read the Exchange Hall sample tap",
          "room_pack": "neighborhood.ceres_freeport",
          "room_entry": "room.c07_exchange",
          "detail": "sample tap",
          "instructions": "Visit the Exchange Hall, use +quest/check on this giver, then return."
        }
      ],
      "completion": "My former captain checked the sample tap and returned to compare the recorded observation with me."
    }
  }
}
```

This is an entries fragment. Declare each room pack's exact installed version
and optional digest pin in the manifest's `requires`. Its applied dependency
must be healthy and active, and the named entry must resolve to a live room with
that public Atlas detail. The engine derives the source identity and room IDs;
authors do not supply `schema`, `key` or numeric `room` fields. A quest may have
up to eight checkpoints, with distinct IDs.

Only one quest source may bind to an NPC. Revisions preserve the NPC's identity,
location, memories and existing progress. Accepted work keeps its frozen
definition; a source edit does not silently change the invitation already
accepted. Rebinding a role to another NPC or retiring a live role is refused.
Write `completion` as the giver's account of what actually happened; it becomes
their verified work experience and is labelled as their account in the journal.

See also: `help passengers`, `help npc`.
