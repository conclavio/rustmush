# 26. Detachments — the unit that leaves a ship together, and how its lead commands it

**Draft 0.1, 2026-10-27.** Written read-only at `00acc546` (`main`), on top of doc 23 Draft 0.2
(PR #35, the seal epoch) and doc 21 §7.3 (cut G.12, the cockpit's keys). Nothing in the tree changed.
This note answers the owner's second sortie question of 2026-10-27: *three players board a ship at
Phobos, fly to a mining field, and disembark in any combination of one to three players with zero
to five AI squads; the player who originates the outing picks their unit, names players who must
accept, and fills the rest with AI squads that embarked with the ship at the port; only the lead
commands the AI squads, and the lead's word to a human is an instruction, not an action.* Doc 23
settled what the engine's "sortie" is — one hull's custody epoch — and left the player's outing
undefined. This note defines it, and it is deliberately not an epoch.

The lead's proposals are D-26-*n*; the owner's questions Q-26-*n* with a default each. Per the
owner's standing word, a default stands provisionally and never holds a build.

## 1. Three things the tree does today that this note has to respect

- **A squad is dealt, not formed.** `crates/tacsim/src/deliberation/squad.rs` deals a side's
  machine units, in unit-id order, into squads of at most `[tacsim.arena].squad_size` (four by
  default); the commander is the lowest id and, since 17b cut L.1, where the NPC lance-leader rides
  (D-UM-53). **A seated pilot's tank is in no squad at all.** So "squad 1 is me" is not a thing the
  arena can say, and an `ORDER` aimed at a human-flown hull has nothing to land on: a lead who
  orders a human gets `No such squad.`, and a non-lead `You have no squad to order.`
  (`crates/tacsim/src/match_/cockpit.rs`, `command_of`).
- **The lead is the lobby's.** `Centre::leads_side` (`crates/tacsim/src/match_/lobby.rs:450`) is
  settled per side over the claimed slots in claim order, and `ORDER` reads it: a lead commands
  every roster of the side; anyone else only their pod pair's tank. The lobby is the arena's and
  doc 23 §1 calls the arena "a proving ground for developing ground combat while the rest of the
  world is built". The rule is right; its home is wrong for the world.
- **The ship already carries people and hulls.** `+ship/board` places a person `Interior` to the
  hull (`crates/tacsim/src/port/ship.rs:864`), a clearance freezes the manifest (IC9), the Dray-P's
  bay holds rigs `Interior` to a bay compartment (doc 20; doc 23 §2's first hangar placement), and
  `+tv/launch` takes one out through the surface excursion host, the host doc 23 picks for the first
  IC sortie (D-23-7). SM.12b's dispatcher already files plans for a house's mind-flown rigs from
  the same bay. Everything a detachment needs to be *carried* is built; what is missing is the
  roster and the command over it.

## 2. Terms

- **Detachment** — a roster of up to six **slots** under a **lead**, formed aboard a carrier,
  that disembarks together. Slot 1 is the lead's own hull. Every other slot is either a **human
  member** (an account, and the hull they will fly) or an **AI squad** (one or more hulls flown by
  minds under one NPC squad leader riding the commander hull, D-UM-53). The object the owner called
  "the unit".
- **The outing** — the detachment's time out: from its first hull leaving the carrier's bay to the
  moment its last hull is back in a hangar, or the lead disbands it. The player's "sortie". **Not an
  engine object**: a record over the member hulls' epochs, exactly as doc 23 makes the voyage a record
  (D-23-2), and nothing about it opens or closes an epoch (D-23-1 stands untouched).
- **An order** — the lead's `ORDER <slot> <objective>`, as the cockpit verb reads today.
- **An instruction** — the same line delivered to a human member: told, shown, acknowledged, and
  never acted on by the sim.
- **The carrier** — the hull whose bay the detachment forms in and leaves from; a Dray-P today.

## 3. The roster — D-26-1

A detachment has at most six slots, numbered once at formation and fixed for the outing. Slot 1 is
the lead. Slots 2–6 are filled in the order they are accepted or filled (Q-26-2 asks whether the lead
may rearrange them before the disembark). The number is what the cockpit's `F2`–`F6` send (doc 21
§7.3) and what `ORDER <n>` names, so a key means the same thing for the whole outing.

A slot holds exactly one of:

- a **human member**: an account (the lobby's one-slot-per-account rule, for the same reason) and a
  hull aboard the carrier that the account may fly — their own, or one the carrier's charter holder
  lends (the `helm_of` title and the lease gate of IC13 decide "may fly" today; the ownership and
  delegation layer of doc 08 replaces that test when it lands);
- an **AI squad**: a set of hulls aboard the carrier, each `Controller::Mind` under a sealed mind,
  grouped as they were embarked (D-26-3), that the lead may command — the carrier's charter holder's
  hulls, or their organisation's.

A hull is in at most one detachment. A person is in at most one. A detachment with no human but the
lead is legal (a lone pilot with five AI squads); one with no AI squad is legal (three friends).

**An AI squad is formed at embarkation, not dealt at the site — D-26-3.** The arena deals squads
because a side of twelve reflex hulls needs commanders and there is nothing else to go on. A carrier's
bay is content: the hulls were put aboard one at a time, and the embarkation is where a squad is
declared — `+ship/embark <hull> [squad <n>]`, a squad being one hull unless the owner says otherwise.
For a machine that makes squad and hull coincide, which is what the owner's "a squad might be another
tank or biped" wants. An infantry squad — six battle armour, eighteen rifle troopers — is a **unit
kind** doc 22 does not have yet: one `TacticalUnit` with a headcount and a look, or a formation of
bodies; Q-26-3 asks which, and nothing here depends on the answer.

## 4. Forming the detachment: the generator — D-26-2

Aboard the carrier, at a port or in the field, in this order:

| Step | Verb (Q-26-1 on the family's name) | What happens | Refused when |
|---|---|---|---|
| form | `+sortie/form <hull>` | the originator becomes the lead of a new detachment; `<hull>` is a hull aboard they may fly, and slot 1 | not aboard; already in one; the hull is out, in another, or not theirs to fly |
| invite | `+sortie/invite <player> [<hull>]` | a pending invite to a person aboard the same carrier, with a hull suggested or left to them; it stands until accepted, declined, withdrawn, the disembark, or its TTL (ten minutes, `HUMAN_ORDER_TTL_MS`'s precedent) | the invitee is not aboard, or in a detachment; no slot left |
| accept / decline | `+sortie/accept [<hull>]`, `+sortie/decline` | the invitee takes the next slot with a hull they may fly, or does not. The survey's `/accept` and `/cancel` idiom (`crates/tacsim/src/port/survey.rs`) is the shape | the hull is taken, out, or not theirs |
| fill | `+sortie/fill [<n>]` | the lead fills the next empty slots, or `n` of them, with AI squads aboard that the lead may command, in bay order | none aboard; none left |
| arrange | `+sortie/slot <n> <member>` | the lead moves a member to slot `n` (Q-26-2) | after the disembark |
| roster | `+sortie/roster` | the detachment as it stands: slot, who, hull, state (aboard, out, docked, down) | — |
| lead | `+sortie/lead <member>` | the lead hands the lead to a human member; the lobby's `leads_side` settle rule is the precedent | the member is an AI squad |
| disband | `+sortie/disband` | the lead ends the detachment; a hull out stays out under its own epoch (doc 23 D-23-1: nothing here closes one) | — |

The detachment lives in the world's state (Redis-authoritative, like the lobby's pods) and survives
a restart; an invite is a RAM row with its TTL. **A clearance freezes it** the way IC9 freezes the
manifest: once `+ship/undock` or a queued `depart` is filed, no slot changes until the carrier is
docked or settled again.

**The AI squads had to embark at the port.** That is not a rule of this note; it falls out of
D-26-1: a slot holds hulls *aboard the carrier*, and a hull is aboard exactly when it is `Interior`
to the bay (doc 23 §2's hangar placement). A squad left on the pad at Phobos is not on the roster
because it is not on the ship.

## 5. The disembark and the return — D-26-4

At a site where the carrier is settled (`Berth::Docked`), `+sortie/disembark` launches every member
hull through the surface excursion host, in slot order, each launch opening that hull's epoch as
doc 23 D-23-1 says, under the site's 32-out cap and `D-20-7` (the carrier cannot leave with a hull
out). A human member's hull launches only with that member seated in it: an unseated human slot is
told and skipped, and may launch alone later with `+sortie/launch`. The lead's disembark is what
sends the AI squads out; a human member may not. The epoch's seal line (`MindSealed`) gains the
detachment's id and slot (D-26-6), so an outing can be reconstructed from the black boxes; the
principal of each seal is as doc 23 D-23-5 already has it — the lead for the AI squads' hulls, each
human for their own.

A member hull returns as any hull does (`+tv/recover` into the bay, staff `Home`, a second carrier's
recovery — doc 23 §3's close kinds), and the detachment marks the slot `docked` or `down`. **The
outing ends** when every member hull is docked or down, or on `+sortie/disband`; the roster is then
history (`+sortie/roster` shows the last one until a new detachment forms).

**A hull's epoch and the outing are different lengths on purpose.** A human who recovers early,
re-arms, and launches again has two epochs inside one outing; the detachment's slot is the same.

## 6. Commanding it: one verb, two deliveries — D-26-5

The lead's `ORDER <slot|ALL> <objective>` is today's verb with today's grammar (HOLD, ADVANCE,
ENGAGE, WITHDRAW, SCREEN, SUPPORT, RELEASE, with `HOLD FIRE`). The slot decides what the line is:

- **To an AI squad it is an order**: the standing order to the squad's leader through the unchanged
  Order path (D-UM-15's re-keying into each mind's view; `HUMAN_ORDER_TTL_MS` as its life). The
  minds weigh it against doctrine as they do now — an order is an input to deliberation, never a
  puppet string — which is the owner's "more inclined to just follow orders" as already built.
- **To a human member it is an instruction**: a line in their cockpit feed in the lead's words
  (`Lead: ENGAGE X2.`), and the frame's existing `order` slot carried with `advisory: true` so the
  cockpit's readout shows *Instructions from lead: ENGAGE X2* beside their own standing state. The
  sim changes nothing on their hull. Two short verbs close the loop to the lead: `WILCO` (the lead
  is told *Slot 3 wilco*) and `NEGATIVE [<why>]`. An instruction stands until replaced, answered
  `NEGATIVE`, or its TTL. `ORDER ALL` delivers each way to each slot.
- **Only the lead issues.** A member's `ORDER` is refused in today's words (`You have no squad to
  order.`). The lead may attach an AI squad to a human member — `+sortie/attach <squad slot> to
  <member>` — who then orders *that squad alone*, the arena's wingman rule (`Command::Wingman`)
  generalised from the pod pair to an attachment; Q-26-4 asks whether to build the attachment in the
  first cut or hold it.

The cockpit's `F1`–`F6` (doc 21 §7.3) are the prefill of this verb and nothing more: the same key
reaches a human and an AI squad, and the delivery is the sim's choice. A non-lead's `F2`–`F6` are
refused in the same words as the typed verb.

## 7. The arena rig, mapped — D-26-7

Until the detachment exists the rig keeps working, because its lobby side already *is* a
detachment with the numbering implied:

```text
slot 1   the side lead (Centre::leads_side)
slots 2… the side's other seated pilots, in claim order        (human members)
then     the side's dealt rosters, in ordinal order             (AI squads)
```

The first cut (DT.1, §10) makes `ORDER <n>` and `command_of` read that one list, so a lead's `ORDER
2` reaches the second human as an instruction and `ORDER 4` the first roster as an order, and the
dealer, the lobby and `SIM` are untouched. Nothing of the rig's lifecycle changes. When DT.2–DT.3
land, a detachment *is* the list and the rig's construction of it is one of two sources.

## 8. What stands on doc 23, and what this note adds to it

- **Nothing here opens or closes an epoch.** D-23-1 is the whole rule and the detachment respects
  it in every line of §5.
- **Nesting by record** (D-23-2) is extended by one field: a `MindSealed` line names the detachment
  id and slot when the hull left as a member (D-26-6). The voyage remains reconstructable, and so is
  the outing.
- **The hangar placements** (D-23-3) are what "aboard" means in §3 and §4.
- **The principal** (D-23-5) is untouched; the lead is the principal of the AI squads' seals because
  the lead's disembark is "the plan the hull leaves under".
- **The hosts** (D-23-6, D-23-7): the disembark is N launches on the surface excursion, so DT.3
  lands after doc 23's S.1, and the ship's own passage (S.2) is the carrier's, not the detachment's.
- **The words** (Q-23-5): doc 23's default is that "sortie" never prints. This note proposes the
  player's word for the outing *is* "sortie" and the verb family carries it (Q-26-1); the engine's
  word stays `epoch` in prose and `SortieId` in code as doc 23 has it.

## 9. What this note does not decide

- **The fight on real ground.** 16c H1 and doc 20 §17's promotion. A detachment disembarking at a
  hostile site is exactly the arrival that promotes the site's ground into an arena; this note only
  guarantees the promotion has a roster to read (the detachment becomes the side, its slots the
  list of §7). Until that ruling, outings are mining, survey and drone outings and the rig's.
- **Infantry** as a unit kind (Q-26-3; doc 22).
- **The detachment net** — a comms channel of its own on doc 15's relay, with the light lag and the
  availability rules that doc already has. Wanted; a cut after DT.3.
- **Ownership and delegation** (doc 08): who may fly whose hull and command whose squads is read
  from the charter title today and from doc 08's roles when they land; this note changes neither.
- **Who pays** when an AI squad's hull is lost on an outing the lead ordered: H1's question, not
  this note's.

## 10. Cuts

Each through the established cut loop (builder, lenses, fixer, gate, judge; one PR per cut off
`main`). Sizes as doc 18 §5.

- **DT.0 — This document, the README row, and the words (S).** Doc 23 Q-23-5's amendment proposed
  (§8); doc 16 §3.7 "Pods" gains the cross-reference; `docs/help/cockpit.md`'s `ORDER` paragraph
  says what a human member receives.
- **DT.1 — The slot list and the two deliveries, in the arena rig (M).** `command_of` and
  `ORDER <n>` over §7's list; the instruction to a human seat (the feed line, the frame's advisory
  `order`, `WILCO`/`NEGATIVE`, the TTL); `ORDER ALL` both ways; the frame's `slots[]` for the cockpit
  legend; stories: a lead orders slot 2 (human) and slot 4 (roster), the human's frame carries the
  advisory and nothing moves, the roster moves; a non-lead's `ORDER 2` refused; `WILCO` reaches the
  lead; the arena hash unchanged by an instruction (it is not sim state).
- **DT.2 — The detachment aboard a carrier (M).** The object and its state slot; `+sortie/form`,
  `/invite`, `/accept`, `/decline`, `/fill`, `/slot`, `/roster`, `/lead`, `/disband`; the
  embarkation's squad declaration (D-26-3); the clearance freeze; stories at Stickney aboard a
  Dray-P with two humans and the house's rigs.
- **DT.3 — The disembark and the return (M; after doc 23 S.1).** `+sortie/disembark` and `/launch`
  through the excursion host; the seal line's detachment field; the slot states; the outing's end;
  stories: a three-human, two-squad disembark under the 32 cap; a human recovers and launches again
  inside one outing; a disband with a hull out leaves its epoch open.
- **DT.4 — Attachments (S; Q-26-4).** `+sortie/attach` and the generalised wingman command.
- **DT.5 — The detachment net (S; after doc 15's availability rules).**

## 11. Decisions proposed here

| # | Decision | Status |
|---|---|---|
| D-26-1 | A detachment is at most six slots under a lead, slot 1 the lead's hull, each other slot one human member (account + hull aboard) or one AI squad (hulls aboard under one NPC leader); numbered at formation, fixed for the outing; a hull and a person in at most one | lead's proposal |
| D-26-2 | It forms aboard the carrier by form / invite / accept / fill, lives in world state, and is frozen by a clearance as the manifest is | lead's proposal, Q-26-1, Q-26-2 |
| D-26-3 | An AI squad is declared at embarkation (one hull unless said otherwise), never dealt at the site | lead's proposal, Q-26-3 |
| D-26-4 | The disembark is N launches on the excursion host in slot order, humans only when seated; the outing ends when every hull is docked or down or on disband; no epoch is opened or closed by the detachment itself | lead's proposal |
| D-26-5 | One `ORDER` verb, two deliveries by slot kind: a standing order to an AI squad, an advisory instruction to a human with `WILCO`/`NEGATIVE`; only the lead issues | lead's proposal, Q-26-4 |
| D-26-6 | `MindSealed` names the detachment id and slot when a hull leaves as a member (extends D-23-2) | lead's proposal |
| D-26-7 | The arena rig's side reads as a detachment: lead, then seated pilots in claim order, then rosters in ordinal order; DT.1 lands on that list with the lobby and the dealer untouched | lead's proposal |

## 12. Questions for the owner

| # | Question | Default |
|---|---|---|
| Q-26-1 | The player's word and the verb family: `+sortie/…`, amending doc 23's Q-23-5 so "sortie" is the outing and never the epoch? | Yes. The owner said "sortie generator"; the engine's epoch has no player-facing word anyway. |
| Q-26-2 | May the lead rearrange slots 2–6 before the disembark (`+sortie/slot`)? | Yes, before the disembark only; the keys must mean one thing for the whole outing. |
| Q-26-3 | An infantry squad: one unit with a headcount, or a formation of bodies? | One unit with a headcount, in doc 22: it is one slot, one order, one seal, and the sim's cost is one hull's. A formation is the look's problem (the doll grid draws eighteen). |
| Q-26-4 | Attachments in DT.1, or held? | Held to DT.4: the first cut proves the two deliveries; the wingman rule covers the pod pair until then. |
| Q-26-5 | May a detachment form in the field (a carrier settled on a site, members already out)? | Form yes (the carrier is settled, so docked), fill and invite over hulls aboard only; a hull out joins on its next recover. |
| Q-26-6 | Who may fill with AI squads: the carrier's charter holder alone, or any member whose own hulls are aboard? | The lead, over the charter holder's and the lead's own hulls; a member's hulls join as that member's attachment (DT.4). |

## 13. Risks

- **The slot list in the rig is two sources once DT.2 lands.** The rig builds it from the lobby; the
  world from the detachment. One function over one list, with the lobby as a constructor, is the
  guard; the risk is a story that assumes the lobby's claim order where the detachment's slot order
  differs.
- **An instruction is not sim state, and must stay that way.** If the advisory order ever folds into
  the arena hash, a human's `WILCO` would change a deterministic replay. DT.1's test pins it out.
- **The bay is RAM-shaped content today.** The embarkation's squad declaration (D-26-3) is a new
  durable fact on the manifest; IC9's clearance freeze must cover it or a squad changes under a
  filed burn.
- **The fight is the point and it is gated.** Every cut here is useful without it (mining outings
  with escorts, drone outings), but the owner should know the Phobos flow's last leg waits on H1.

## 14. Ledger

*(Empty.)*
