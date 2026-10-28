# Quests

Some NPCs offer optional work and introductions. Conversation can introduce
them, but the quest service checks eligibility, records the work and confirms
completion. You can use the commands with the model disabled. These invitations
are independent: accepting one never requires another, and the services they
explain remain available without accepting a quest.

  +quest [<giver>]                 Read your own progress or available work.
  +quest/offer <giver>             Read the invitation and its acceptance token.
  +quest/accept <giver>=<token>    Accept the exact invitation printed by the service.
  +quest/check <giver>             Record an objective at its required location.
  +quest/finish <giver>            Return to the giver and share completed observations.

Acceptance and finishing require the giver to be present and mutually
perceivable. A check requires you to be in the objective's named room. The
service verifies the actual work: saying you did it does not record a check.
Your progress and service answers are private to you. Use the acceptance command
printed by the offer, then read `+quest` to confirm your committed journal.

Finishing grants **+10 personal trust and +5 disposition with the giver**, once
per quest. There is no quest credit or faction-standing reward. Normal market
payments, passage fares and survey commission rewards keep their own rules.
Restarting or repeating a command cannot repeat the completion bonus.

## Independent invitations at Ceres

| Giver and location | Invitation | What it verifies |
| --- | --- | --- |
| Ilse Oduya-Brandt, Registry & Co-op Desk | The papers behind a captain | Your own active Co-op enrolment, real charter and stake payment. |
| Dov Akintola, Co-op Chandlery | A small useful cargo | A fresh purchase of at least one tonne of crated fittings (`industrial_parts`) at Stickney, then a delivered sale to the Ceres Co-op. |
| Yusra "Ma" Petrossian, the Slack Line | Someone at the next table | Meeting Nadia Okafor-Sato in person and learning her public route. |
| Rosalind Qureshi-Mbeki, Exchange Hall | Wide first, then close | A fresh ordinary surface scan and sounding on a real site. |

At the respective desk or bar, use `+quest/offer Ilse`, `+quest/offer Dov`,
`+quest/offer Yusra` or `+quest/offer Rosalind`. These first names work as object
name prefixes; the service also prints exact commands using the giver's object
number.

Ilse's invitation is a reason to read `+coop` and decide whether the First
Captain programme suits you. Enrolment is once per account; never enrol just to
clear a journal entry. A current qualifying enrolment can already exist when
you accept. `+coop/statement` reads your record, and `+quest/check Ilse` verifies
it at her desk.

Accept Dov's invitation before buying the fittings. At Stickney, `+market/list`,
`+market/quote <listing>=1` and `+market/accept <quote>` make a real purchase.
Plan fuel and provisions for the trip with `+ship/plan ceres`. At Ceres's
Chandlery, inspect the live bid and use `+market/sell <listing>=1`; let delivery
and payment finish before `+quest/check Dov`. Old cargo or an unaccepted quote
does not prove this work. Prices and demand are live, so no profit is promised.

Ma's introduction does not require booking a passage. Nadia initially sits in
the Slack Line, south of the Freeport Concourse. `+passage Nadia` shows her
current situation; use `+quest/check Yusra` with Nadia present to record the
meeting. She is one persistent person: she may already be booked or have left,
and the invitation never summons or resets her. A fare discussion through
`+passage/offer Nadia` is a separate choice.

Accept Rosalind's invitation before the scan and sounding. `help mining`
explains the licence, refit and rig lease; choose a real site with
`+chart/ground`. Preview and submit the actual rig operations with `+tv/1 scan`,
`+tv/1 scan go`, `+tv/1 sound` and `+tv/1 sound go`. Successful manual or planned
operations count; quiet soundings also count. Both readings can be taken on the
same trip. Return to the Exchange and use `+quest/check Rosalind` twice, in the
journal's order, then `+quest/finish Rosalind`. Your chart stays private. This
practice needs no paid survey commission or finite commission-board slot.

After any invitation's checks, return to its giver and use `+quest/finish`.

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

The check records the sample tap's current public description. Merely looking
at it or remembering an old description does not record the observation.
Finishing gives Leila one verified work experience and the same once-only
personal relationship bonus. Another character cannot claim your passage or
your progress, even if they have the same name.

## Authoring a quest

A `quest` entry binds to an ordinary `npc` entry. Omit `npc_pack` for a giver in
the same pack, or name an explicitly required pack to use its existing NPC.
External binding adds only the `quest` interface and `+quest` command reference,
preserving the profile's other services, personality, affiliation and ownership.
For a same-pack giver, include those capabilities in its authored profile.

The default prerequisite, `paid_passage`, requires that giver's completed, paid
passage with the asker to `destination`. Use `"prerequisite": "open"` for an
independent invitation at that port. Let the service supply the asker's actual
invitation and progress; do not put completion claims into public lore, persona
or fallback text.

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

This is an entries fragment. Declare each referenced room or NPC pack's exact
installed version and optional digest pin in the manifest's `requires`. Its
applied dependency must be healthy and active. The engine derives the source
identity and object IDs; authors do not supply `schema`, `key` or numeric `room`
fields. A quest may have up to eight checkpoints, with distinct IDs.

An omitted `objective` means a local public Atlas `detail`, as above. Other
checkpoint objectives use these source shapes:

```json
{"kind": "coop_enrolment"}
{"kind": "trade", "origin": "stickney", "destination": "ceres", "commodity": "industrial_parts", "quantity": 1}
{"kind": "passenger", "npc_pack": "tacsim.passengers.ceres", "npc": "npc.nadia_okafor_sato"}
{"kind": "survey", "mode": "scan"}
{"kind": "survey", "mode": "sound"}
```

Each still needs its title, instructions and bound `room_pack`/`room_entry`:
that is where the player acknowledges the verified work with `+quest/check`.
`detail` may be omitted for these objectives. A passenger objective verifies a
local meeting with the real traveller, not a booking or completed passage.
Trade and survey evidence must come from actual work after acceptance. Use
honest current availability when a shared traveller has already left.

Only one quest source may bind to an NPC. Revisions preserve the NPC's identity,
location, memories and existing progress. Accepted work keeps its frozen
definition; a source edit does not silently change the invitation already
accepted. Rebinding a role to another NPC or retiring a live role is refused.
Write `completion` as the giver's account of what actually happened; it becomes
their verified work experience and is labelled as their account in the journal.

The independent packs `tacsim.passengers.ceres@1` and
`tacsim.quests.first_steps@1` require Ceres v4 and Stickney v4. New worlds load
them in dependency order. Existing worlds with those versions install them at
boot; worlds with older station content wait for staff to plan and apply the
dependency upgrades. No existing station pack is automatically upgraded.

See also: `help passengers`, `help npc`.
