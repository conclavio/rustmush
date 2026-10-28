# Quests

Some NPCs offer optional work and introductions. Conversation can introduce
them, but the quest service checks eligibility, records the work and confirms
completion. You can use the commands with the model disabled. Starter invitations
are independent; some later favors require an earlier completed quest or offer
a choice of paths. The invitation states those terms. Ordinary services remain
available without accepting a quest.

  +quest [<giver>[=<quest-key>]]    Read your own progress or available work.
  +quest/offer <giver>[=<quest-key>] Read eligible invitations or one exact offer.
  +quest/accept <giver>=<token>    Accept the exact invitation printed by the service.
  +quest/decline <giver>=<token>   Permanently decline that exact invitation.
  +quest/check <giver>[=<quest-key>] Record an objective at its required location.
  +quest/finish <giver>[=<quest-key>] Return to the giver and finish that work.
  +quest/abandon <giver>[=<quest-key>] Permanently abandon accepted work, even remotely.

Acceptance and finishing require the giver to be present and mutually
perceivable. A check requires you to be in the objective's named room. The
service verifies the actual work: saying you did it does not record a check.
Your progress and service answers are private to you. Use the acceptance command
printed by the offer, then read `+quest` to confirm your committed journal.
An NPC can offer several quests. Use the full quest key printed by the service
when a command needs to distinguish them; it never guesses between active jobs.

Declining or abandoning is terminal for that quest and gives no reward or
relationship penalty. Decline an unaccepted offer with its exact token; abandon
work you already accepted. Neither command undoes ordinary trades, bookings or
other gameplay you performed along the way.

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

## Work worth signing

After finishing **Wide first, then close**, Rosalind can offer a follow-up about
completing paid survey work. At the Exchange, ask:

  +quest/offer Rosalind=tacsim.quests.survey_work/quest.qureshi_signed_work

Choose any available job on `+survey`. Read its terms with `+survey <id>`;
`+survey/accept <id>` previews and `+survey/accept <id> go` files it. Wait for
funding, then complete fresh soundings of all 256 pockets in its block within
seven IC days. `help mining` explains equipment, coverage and quiet soundings.
An unfinished commission may already exist when you accept the quest. The
commission must finish after quest acceptance; old completed work cannot count
even if its payment arrives later.

Once the ordinary commission payment settles, return to the Exchange and use:

  +quest/check Rosalind=tacsim.quests.survey_work/quest.qureshi_signed_work
  +quest/finish Rosalind=tacsim.quests.survey_work/quest.qureshi_signed_work

The board has four shared, finite jobs. This invitation reserves none of them,
and is unavailable when there is no eligible unfinished or available commission.
Partial work, cancelled jobs and refunds cannot satisfy it. Your chart stays
private. The quest adds Rosalind's once-only personal relationship bonus;
Occator's ordinary payment and reputation effects are separate. Commissions
remain available without either of her quests. Abandoning the quest leaves any
commission in place; cancelling one uses the ordinary `+survey/cancel` command.

## People who keep the port moving

Ma also offers an independent introduction to two people at Ceres: Dov at the
Co-op Chandlery, then Rosalind at the Exchange Hall. Ask for her exact offer:

  +quest/offer Yusra=tacsim.quests.port_connections/quest.ma_port_connections

Visit each person in order and use the `+quest/check` command in your journal,
then return to Ma. Each check verifies that you and the named NPC are actually
present and can see one another. It records a meeting, not a conversation you
did not have. You can chat with them about their work and services, but buying,
enrolling, surveying or accepting another quest is optional.

This introduction works alongside Ma's invitation to meet Nadia. Use the full
quest key if both are active. A person who has left, is hidden or is unavailable
cannot be credited through a same-named substitute.

## See her safely there

If a trip to Stickney fits your plans, Ma can offer a separate favor involving
Nadia's passage. Read Nadia's actual situation with `+passage Nadia`, then ask:

  +quest/offer Yusra=tacsim.quests.safe_passage/quest.ma_safe_passage

Accept before Nadia arrives at Stickney. A passage you have already booked, or
one with Nadia already aboard your ship, can count if arrival is still ahead.
The favor is unavailable once she has arrived or while she is booked with
another captain. She remains the same shared traveller; this quest never
reserves her for you or creates another passenger.

Negotiate, book and board her through the ordinary `+passage` commands. Use
`+ship/plan stickney` to check your route, fuel and provisions. After delivering
her and receiving the settled fare, return to Ma at the Slack Line and use:

  +quest/check Yusra=tacsim.quests.safe_passage/quest.ma_safe_passage
  +quest/finish Yusra=tacsim.quests.safe_passage/quest.ma_safe_passage

Arrival alone does not prove payment, and a journey completed before accepting
does not count even if its fare settles later. The quest adds Ma's usual
once-only personal relationship bonus. Nadia's fare, agreement and destination
keep their ordinary rules. You can carry her without this favor, and abandoning
the favor does not cancel a booking or change your passenger obligations.

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

## Leila's next piece of work

After you complete **A second reading**, meet Leila again at Ceres's Visiting
Berth Gallery and use `+quest/offer Leila`. She offers two alternatives:

- **A useful consignment**: after accepting, buy at least one tonne of fittings
  at Stickney and complete its delivered sale to the Ceres Co-op. Acknowledge the
  trade at Dov's Chandlery with
  `+quest/check Leila=tacsim.quests.leila_followup/quest.leila_trade`.
- **Two scales of looking**: after accepting, complete an ordinary surface scan
  and sounding, then acknowledge each in order at Rosalind's Exchange desk with
  `+quest/check Leila=tacsim.quests.leila_followup/quest.leila_survey`.

These are Leila's invitations even though you record the work at another desk.
Return to her to finish, using the same full quest key with `+quest/finish`.
The trade has ordinary live prices and payment; the readings need no paid
commission and disclose no private chart. Each path gives Leila its own account
of the work you completed together.

**Accepting either path permanently chooses it.** The other path closes, even
if you later abandon the accepted work. Declining one offer before accepting
does not choose a path, so the other can remain available. You may decline both
or do neither. This choice affects only Leila's follow-up: Dov's and Rosalind's
starter invitations, ordinary trade and ordinary surveying remain independent.

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

For a follow-up, use an exact completed source key and, optionally, a choice:

```json
"prerequisite": {"completed": {"quest": "tacsim.passengers.stickney/quest.leila_second_reading"}},
"choice": {"group": "tacsim.leila_next_work", "path": "trade"}
```

The prerequisite belongs to the current character's durable completed journal.
Use a stable, namespaced group and distinct path identifiers for alternatives.
Acceptance reserves that group permanently; declining an offer does not.
Choice groups are supported on open and completed-quest prerequisites. Multiple
later quests may share the chosen path; a different path in that group stays
unavailable. Keep groups specific to an authored choice, not ordinary services.

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
{"kind": "contact", "npc_pack": "neighborhood.ceres_freeport", "npc": "npc.akintola"}
{"kind": "paid_passage", "npc_pack": "tacsim.passengers.ceres", "npc": "npc.nadia_okafor_sato", "origin": "ceres", "destination": "stickney"}
{"kind": "survey", "mode": "scan"}
{"kind": "survey", "mode": "sound"}
{"kind": "survey_commission"}
```

Each still needs its title, instructions and bound `room_pack`/`room_entry`:
that is where the player acknowledges the verified work with `+quest/check`.
`detail` may be omitted for these objectives. A passenger objective verifies a
local meeting with the real traveller, not a booking or completed passage.
Trade and survey evidence must come from actual work after acceptance. Use
honest current availability when a shared traveller has already left.

A `contact` objective can name any authored NPC, including ordinary station
staff. Bind the NPC's pack as an explicit dependency and choose the room where
the meeting should happen. The live NPC must be enabled, mutually perceivable
and present with the player when the check commits. Previously acknowledged
meetings remain historical receipts if that NPC later moves. Contact checks do
not assert that dialogue happened, transfer items or sign up for services.

A `paid_passage` objective names an existing traveller and exact route. Its room
is where the player reports the completed journey; the traveller does not have
to return there. It requires the player's actual delivered booking and exact
settled fare receipt. Delivery must occur after quest acceptance; an unfinished
booking may already exist. Acceptance rechecks availability, and recording or
finishing rechecks the authoritative booking and payment. A changed booking,
unsettled or reversed fare, another captain's trip or old arrival cannot grant
completion. Declining or abandoning the quest never mutates the passage.

A `survey_commission` objective accepts any real commission from the finite
survey board. Its room is where the player reports the work. It requires all
256 pockets completed by this character after quest acceptance and the exact
settled reward receipt. An unfinished commission may predate the quest. The
service rechecks the authoritative commission and unreversed payment when
recording or finishing; a changed row, partial work, refund, another pilot's
work or old completion cannot grant credit. The invitation checks current board
availability without reserving a job. It neither reveals private chart data nor
changes ordinary commission payments or standing.

Up to eight quest definitions may bind to one NPC, within a 64 KiB catalog.
Definitions merge by full pack/entry key and preserve other packs' definitions.
Existing single-definition attributes still load; catalogs of multiple
definitions use a sorted array. The whole incoming pack is checked against the
limit before any capabilities are added. Invalid or non-source-owned catalogs
are refused rather than replaced.

Revisions preserve the NPC's identity, location, memories and existing progress.
Accepted work keeps its frozen definition; a source edit does not silently
change the invitation already accepted. Rebinding a source to another NPC or
retiring a live source is refused.
Write `completion` as the giver's account of what actually happened; it becomes
their verified work experience and is labelled as their account in the journal.

The independent packs `tacsim.passengers.ceres@1` and
`tacsim.quests.first_steps@1` require Ceres v4 and Stickney v4. New worlds load
them in dependency order. Existing worlds with those versions install them at
boot; worlds with older station content wait for staff to plan and apply the
dependency upgrades. No existing station pack is automatically upgraded.
`tacsim.quests.leila_followup@1` separately requires Leila's passenger pack v2
and both stations at v4. It adds two definitions to the same Leila without
replacing her earlier quest or resetting her journey.
`tacsim.quests.port_connections@1` requires Ceres v4 and adds Ma's optional
introductions without changing any station NPC's identity or ordinary services.
`tacsim.quests.safe_passage@1` requires Ceres v4 and Nadia's passenger pack v1;
it adds Ma's passage favor to her existing catalog.
`tacsim.quests.survey_work@1` requires Ceres v4 and the first-steps pack v1;
it adds Rosalind's paid-work follow-up without changing the commission board.

See also: `help passengers`, `help npc`.
