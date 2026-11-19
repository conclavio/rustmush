# 50. Law and the corridor — control that sets terms, a faction operation players feed, and offenses with consequences, surveyed

Draft 0.4, 2026-11-19, docs only. Surveyed against `origin/main` at `1e9b41174` by five read-only
lanes (fronts and clocks; the charges on a run; customs, offenses and standing; faction money and
jobs; the prior rulings and the setting), with the lead spot-checking the claims §0 leans on:
- `record_customs_assertion`'s whole path (six segments opening at one, the consequence text, the
  front resolved, no control flip);
- `fire_consequence`'s control flip, which that path does not use;
- doc 11 §5's Kirkwood and Isidis rows and §6's three corridor fronts;
- the venue fee, berth and haul bond constants, and the lane's `authority` rows;
- the market's `Legality` field and its own comment;
- port security's two hard-coded ports and three fines;
- the picket seizure's debit;
- the campaign crate's objective and consequence kinds and its notice bounds;
- that `crates/tacsim/src` calls nothing in `mush-campaign`;
- that Isidis is prose in four content files and nothing else;
- the rulings quoted in §4.2.

Anchors are file names with line numbers; line numbers drift. Draft 0.1, 2026-11-18, was the
survey.

**Draft 0.2 folds in an outside review** of the PR at `d4ff2da20`, triaged in
[doc 50a](50a-outside-review-triage-2026-11-19.md) with five verification lanes against the same
tree. Of about 55 claim parts, about 40 held, about a dozen held in part and four were refuted.
- **The owner ruled the same day:** "defaults not addressed specifically by the reviews are
  accepted as written". That rules eleven questions, each marked **Ruled** in §6: Q-50-3, -5, -11,
  -12, -13, -14, -41, -45, -46, -48 and -49.
- **Facts the survey had wrong are corrected in place**, marked **Correction (outside review, doc
  50a …)** with the triage row: §0, §3.1 to §3.4, §3.6 and §4.2.
- **§4.3 gains eleven live defects** the review found, for the fix thread.
- **Thirty-four defaults are corrected in place.** Four that the review cited are unchanged:
  Q-50-8, -15, -16 and -17.
- **Choices the findings need** are the follow-ups Q-50-50 to Q-50-71 (§6.6).
- **The model of §5 stands.** The largest change is to Q-50-18: the campaign engine cannot frame
  the Isidis operation as built, and Q-50-57 lists what it must gain.

**Draft 0.3 records the owner's second ruling**, made the same day. Asked whether to accept the
open defaults and follow-ups apart from five held for his own call, he answered "Accept those".
- **Fifty-five more questions are ruled**, each marked **Ruled** in §6: the four the review
  cited and left unchanged, as written; the 34 corrected in place, as corrected; and seventeen
  of the follow-ups, as proposed. With the first ruling's eleven, 66 of 71 are ruled.
- **Five were held for the owner's own call:** Q-50-55, -57, -62, -65 and -67. Draft 0.4 rules
  them.
- **Two earlier documents are answered by these rulings.** Q-50-6 reverses Q-48-15's "No
  customs duty" where a holder levies one, and Q-50-52 takes doc 11 §9 question 2 at its
  recommended answer. Each of those documents carries a note.
- No default's text changed in that draft.

**Draft 0.4 records the owner's third ruling**, also made the same day. Shown the five with
each default and its alternative, he answered "Take all five defaults".
- **All 71 questions are ruled.** Q-50-55, -57, -62, -65 and -67 are accepted as proposed.
- **`mush-campaign` stays the frame of the operation** (Q-50-57), with its five additions built
  as their own unit first. That answer is marked expensive to reverse.
- **Q-50-26 is built for the first operation**, because Q-50-62 gives a hull a way to break
  off.
- **Nothing waits on an open question now.** §6 says what the ruling settled in the answers
  that had waited.
- **No default's text changed in this draft.** Units are still cut only on the owner's word.

This is a survey in the form [doc 38](38-after-the-ic-phase-next-systems.md) §5,
[doc 44](44-minds-wired-through-the-game.md) and [doc 48](48-economy-and-production.md) used. It
sets out what exists, then asks the owner questions (§6), each with a proposed default, an
alternative and a reason. **Provisional** marks a number to be tuned later, under the 2026-10-24
rule that economy defaults are taken and tuned, never waited on. **(expensive to reverse)** marks an
answer that would take a significant refactor of the codebase to undo; stored data and live boxes
do not count (Q-38-9, Q-38-10). §7 sketches the units of work; they are cut only after the rulings.

Combat is described at the level of game mechanics throughout (the owner's rule of 2026-11-02).
The game speaks American English (the owner's ruling of 2026-11-17), so this document does too,
except where it quotes an identifier or an earlier document.

## 0. In brief

1. **The ask.** Docs 48 and 49 gave players reasons to care what a shipment, an escort, a fight and
   a replacement hull cost. Doc 50 lets those decisions change what another player meets tomorrow.
   It has three parts:
   - the Kirkwood customs front gets a mechanical outcome that reaches a price;
   - a faction operation with a budget and a stockpile objective decides who administers Isidis;
   - one restricted good at one port gets jurisdiction, evidence, and a penalty a player can see
     and resolve.
2. **Control is built and unused.** The faction engine has regions and typed control records (de
   jure, de facto, covert, economic), and a clock that can flip one when it fills. TacSim seeds no
   region and no control record, and reads none. Doc 11 §5 already tabulates who holds each place
   in exactly those four kinds.
3. **The Kirkwood front fills into text.** A fine, a seizure or a picket run each advance "Customs
   Assertion". The sixth incident appends one sentence to a canon record and marks the front
   resolved. Nothing reads that. The picket, its fine and every price are the same the day after.
   The front cannot reopen.
4. **Nothing on a run is taxed.** A hauler meets a listing fee, a sale cut, a levy, a berth fee, a
   locker fee, the Co-op's share and a contraband fine. There is no duty, toll or tariff on
   ordinary cargo anywhere, by an earlier ruling (Q-48-15: "No customs duty"). So "Republic fees"
   has nothing to raise, and "Belt cargo lands tax-free" is the only state there is. The prize
   needs the tax to exist.
5. **No price says why.** No quote, sale line or fee names its cause. `+market/history` shows
   house prices by IC week and nothing more.
6. **The operation engine is built and unreached.** `mush-campaign` declares an operation with a
   held bond, a notice of 28 to 84 IC days, a window, sides and stakes, and settles into one canon
   record that a faction clock can turn into a control flip. No TacSim code calls it. Its
   objectives are a closed window, a muster count, or a named canon record; there is no stockpile.
   **Correction (outside review, doc 50a A-2):** that record does not say who won to anything that
   watches it, only a staff command can declare, and organizations cannot be sides. The engine
   cannot frame this operation as built (§3.4, Q-50-57).
7. **A faction cannot post a job.** A haul or an escort is posted by a character standing in a
   port, from their own locker to their own locker, paid from their own account. No organization
   and no NPC can post one. The haul code handles no wreck, capture or seizure.
8. **Isidis is prose.** It is named in two NPCs' facts and two rooms' text. There is no port, site,
   region, organization or front for it. Two of the three fronts doc 11 gives the corridor (the
   Concession Review and Hold the Concession) are not in the code.
9. **Legality is a field nothing sets.** The market records legal, restricted or forbidden per
   catalog entry, one for a good everywhere, and every TacSim catalog row hard-codes legal. The
   venue gate does not see the good. **Correction (outside review, doc 50a A-3):** Draft 0.1 said
   "per listing". A listing has no such field, so the field cannot differ by venue. The market's
   proceeds policy does see the seller, the venue and the good, and may refuse (§3.6).
10. **Enforcement exists in three separate places.** The picket fines or seizes unlicensed
    strategic ore. Port security fines a proven salvage offense at docking, and holds the hull
    once the fine is 28 IC days past due. The Registry sanctions an ungoverned core. Each has its
    own record, its own proof and its own payee, and two of them name their ports in a constant.
    **Correction (outside review, doc 50a C-2):** Draft 0.1 left out the 28 days of grace.
11. **Standing is recorded and nearly unread.** Field fire, capture, survey work and published
    incidents write it. An organization policy test and the NPC prompts read it. No price, picket,
    license or venue does.
12. **Three earlier rulings bind the design.** Control changes only through a clock's typed cause
    or a staff action, never by an LLM. Enforcement acts only on acquired evidence, never an
    automatic omniscient fine. A change in what a player or organization holds is an operation
    with at least 28 IC days of notice.

**The proposal (§5):** one model for all three parts. Control of a region is the faction engine's
record. A short authored schedule turns who holds a region into the terms there: duties, what is
restricted, how often a hold is audited, what a fine costs. Fronts and operations change control;
terms and law follow it. Everything else is wiring that reuses what docs 41, 45 and 48 built.

## 1. Why this document exists

On 2026-11-18, with doc 49 closed and doc 48 two units from closed, the owner asked for the design
of what comes next, in these words:

> Docs 48 and 49 give players reasons to care about the cost of a shipment, an escort, a fight, and
> a replacement hull. The next step is to let those decisions change what another player encounters
> tomorrow. That also completes the original TacSim proving story, which calls for a ground
> operation that changes control or access.

The owner named three pieces of work.

**The Kirkwood customs front.** "Inspections, seizures, and ships running the picket already
advance Customs Assertion. When its clock fills, the code records the consequence as text and
closes the front. The setting gives it a mechanical outcome: the Republic gains effective control
of the approach, Republic fees apply, and Co-op rates rise. Wire that through to an actual quote. A
hauler making the same run before and after should see a different price and be able to find out
why."

**The Isidis Concession.** "Give a faction a budget and a stockpile objective. Let players take
paid haul and escort work, and let deliveries or losses affect the declared operation. The prize is
already written: whoever administers Isidis decides whether Belt cargo lands tax-free. The
milestone I'd use is that a player who never joined the operation can arrive afterward and tell who
won from the terms they're offered."

**Jurisdiction, evidence and faction standing.** "Start with one good whose sale is restricted at
one port. Let an inspection or witness report establish what happened, then give the player a fine,
a standing change, or a restriction they can see and resolve."

This document is the crime design that doc 48 set aside (48 §2: "contraband and legality
enforcement (a crime document; legality is recorded and unenforced)") and doc 45 named as not
designed (45 §6).

It is not a bug list. §4.3 names the defects the lanes found and leaves them to the fix thread.

## 2. What is in scope

**In:**
- who holds a place, and what that changes in the terms a player is offered there;
- the three corridor fronts of doc 11 §6: Customs Assertion, the Concession Review and Hold the
  Concession;
- one declared operation, its budget, its stockpile, its jobs and its outcome;
- what is lawful to sell where, how an offense is established, and what follows.

**Out:**
- facility assaults, infantry and territorial warfare (doc 38, "Out of scope by ruling"; doc 36
  §5);
- a mission generator beyond the raid director (doc 38, the same list);
- the perception or epistemic engine: who knows what stays as it is, and this design uses only
  records a party already holds (§3.6);
- interrogation, ransom and bounties on persons (Q-38-22);
- the mind-commerce families' own sanctions, which stay closed (doc 46);
- the other fronts of doc 11 §6 (Registry Divergence, The Slowdown, the relay fronts): see Q-50-49;
- house policy as a Rhai doctrine (Q-44-32, deferred): the institutions here are compiled arms.

## 3. What exists

### 3.1 Fronts, clocks and control

All of it is in `crates/mush-faction` (`src/sim.rs`, `src/lib.rs`, `src/data.rs`).

- **A front** is `{id, faction, title, resolved}`. **A clock** has segments and a filled count, a
  visibility, an optional canon kind it watches, a consequence string, an optional
  `on_complete_control` of `(region, kind)`, and a `completed` flag. A clock counts up and stops.
  There is no countdown and no opposed clock; the engine's model of opposition is a second clock
  that flips control back (`sim.rs:13-14`).
- **A region** and **a control record** `{region, faction, kind, strength, contested_by,
  visibility}` exist, with the kinds `DeJure`, `DeFacto`, `Covert` and `Economic`
  (`data.rs:426-513`). `+territory` shows them.
- **A clock advances** in two ways. `advance_clock` serves staff `@clock/advance` and the canon
  observer, which is gated to events naming a member of the faction. Hand-written owner functions
  bypass it and set the count directly: `record_customs_assertion`, `record_relay_discovery`,
  `advance_relay_terms_front`.
- **On fill, the engine's path** (`fire_consequence`, `lib.rs:274`) writes the control record when
  the clock names one, then commits a `front.progress` canon record whose summary is the
  consequence string.
  - **Correction (outside review, doc 50a A-1):** the write does not displace. It removes only the
    completing faction's own earlier record of that kind and appends the new one
    (`lib.rs:285-301`), so a region whose de facto holder was another faction ends with two de
    facto records and no marker of which is current. `+territory` would print both.
  - The record's `since` is wall-clock time, not an IC date, and no code reads it.
  - This path does not mark the front resolved. The hand-written paths do, and never flip control.
  - A filled clock cannot be set back and a resolved front cannot be reopened: `@clock/set` takes no
    count, and a clock on a resolved front is skipped (`sim.rs:103-113, 153-162`).
  - Control is written by staff verbs only (`@faction/control`, `/uncontrol`, `/contest`). The
    crate has no public function that writes it.
- **Fronts are made** by staff verbs or by Rust seed functions. No content file defines one.
- **TacSim seeds no region and no control record**, and no file outside `mush-faction` and
  `mush-campaign` names `ControlRecord`. TacSim uses the faction crate for standing, stances and
  the fronts below.
- **The polities are not content.** Each lane organization carries a `faction` tag (`freeport`,
  `republic`, `concord`, `syndicate`, `company`, `townships`) that is "carried and validated, read
  by nothing until the faction cut (lead ruling L16)" (`port/def.rs:1081-1083`). The launch
  bring-up makes one faction by hand, the Republic, with no backing organization, and makes the
  three Houses as organizations.
  - **Correction (outside review, doc 50a C-3):** Draft 0.1 gave five tags and read as if the
    Houses were made as factions. There are six tags, on 24 organizations.
  - **Correction (outside review, doc 50a A-10a):** a faction has at most one backing organization
    and an organization backs at most one faction (`data.rs:301`, `econ.rs:89-92`). Several lane
    organizations cannot be bound to one polity through that field.
  - **Correction (outside review, doc 50a A-2e):** the canon observer advances a clock only when
    the record names a member of the clock's faction, for every kind but `minds.unwarranted`
    (`lib.rs:236-240, 1236`). It matches the record's kind alone, not its source or its detail.

Six fronts ship. None has a mechanical outcome.

| Front | Segments | Advanced by |
|---|---|---|
| Customs Assertion | 6, opens at 1 | the picket's fine, seizure or run |
| The Exchange congestion board | 4, opens at 1 | `scene.concluded`, members only |
| The Concord's anti-proliferation campaign | 8, opens at 1 | a relay discovery |
| The Red Dies | 6, from 0 | `minds.unwarranted` |
| Terms of Service | 4, from 0 | the relay billing case, in jumps of two |
| A Half Afloat | 8, from 0 | the mobile relay's steps |

The Concession Review, Hold the Concession, Registry Divergence and The Slowdown are in doc 11 §6
and not in code. One NPC says "the Concession Review clock stands at 2 of 6"
(`content/cast/stickney_yards.json:50`); no such clock exists.

### 3.2 The Kirkwood customs front as built

- **The inspection** reads the hull's hold and the pilot's papers (`space/picket.rs:682-764`).
  Strategic ore is unlicensed unless the pilot or principal holds both papers: a Freeport Registry
  prospecting license and a registered claim. Nothing else is contraband there.
  - **Correction (outside review, doc 50a C-1):** Draft 0.1 read the test as "no license and no
    claim". It is `!licensed || !has_registered_claim` (`picket.rs:760`): both are needed.
  - The claim is a live claim on a strategic rock in an orbital cluster. A hull that mined must
    hold the claim on every block it mined; a hull that did not needs any one such claim. A surface
    claim never counts. So ore that was bought clears only for a licensed holder of such a claim.
  - **Correction (outside review, doc 50a A-4):** the hold is read through `read_pool`, which for a
    bulk good returns the pooled balance and ignores lots (`port_tick.rs:2610-2615`). Ore held as
    an exact lot reads as nothing. A buyer of a personal lot ask receives such a lot in the hold.
- **The ruling** is one of Cleared, Fined, Seized or Ran, written by the action
  `tacsim_space_customs` (`space/customs.rs`).
- **The fine** is 500 cr a tonne begun, never under 1,000 cr, chosen when the pilot can pay. It is
  paid to the **Freeport Registry**, although the line the pilot reads names a Republic Customs
  corvette (`picket.rs:1402-1492`, `customs.rs:834`).
- **The seizure**, when the pilot cannot pay, debits the ore from the hold in the ruling's own
  transaction. It is credited to nobody (`customs.rs:466-484`). It takes only ore that is free of
  any hold, so cargo under a haul's reservation cannot be seized (doc 50a A-5).
- **A run** (`+helm/run`, or burning past 35 km) turns the corvette hostile.
- **The picket sits on one leg.** The lane's only interdicted leg is the first leg of
  `ceres_stickney`, Ceres to Kirkwood. The outward run and the long way round by Luna and Deimos
  carry none (`ceres_phobos.json:2384-2423`; doc 50a B-3a).
- **The front** (`mush-faction/src/lib.rs:1011-1079`):
  - It is made by the first incident, not at world start, on the board of the license issuer's
    faction, which is the Freeport Registry's. Doc 11 calls it a Republic front.
  - Its clock has six segments and opens at one. Doc 11 §6 says four at one.
  - Every outcome but Cleared advances it. Doc 11 says "each successful interdiction … or a paid
    inspection".
  - On the fill it appends "Republic Customs tightens Kirkwood picket inspections and seizes
    unlicensed strategic ore shipments." to that incident's canon summary and sets the front
    resolved. `on_complete_control` is `None`, and the engine's `fire_consequence` is not called.
- **Afterward nothing changes.** The picket, the ruling and the fine never read the front. Later
  incidents still commit canon. The front is found by title and never made again.
- **No standing moves** on any customs outcome.
- **The approach has no record of its own.** It is the flag `kirkwood_picket: true` on a lane node
  (`ceres_phobos.json:2135`).

### 3.3 What a hauler is charged today

| Charge | Rate | Set in | Paid to |
|---|---|---|---|
| Buying a house's ask | none | — | — |
| Listing fee | 1%, floor 10 cr | `mush-market/src/fees.rs:29`, `port/berth.rs:41` | the port's `authority` |
| Sale cut | 3% | `fees.rs:31` | the port's `authority` |
| Levy | 1% | `fees.rs:33` | the port treasury, a sink |
| Berth | 5 cr an IC day a hull | `port/berth.rs:39` | the port's `authority` |
| Locker | 1 cr a tonne an IC day | `port/locker.rs:44` | the port's `authority` |
| Co-op share | 250‰ of the margin over reference | lane `share_permille`, frozen at enrollment | the Co-op |
| Haul bond | 25% of declared value | `services/haul.rs:78` | held, returned on delivery |
| Contraband fine | 500 cr a tonne, floor 1,000 | `space/picket.rs:86-89` | the Freeport Registry |
| Captive release | 1,500 cr | lane `release_fee_cr` | the port's `authority` |
| NPC crew wage | 10 or 20 cr an IC day a hire | `detachment/crew/wages.rs:36-38` | the hiring port's `authority` |
| Prospecting license | the lane's `licence` block | `port/licence.rs` | the license's issuing organization |
| Port security fine | 500, 2,000 or 5,000 cr | `surface/security.rs:40-46` | the Freeport Registry |

- **Correction (outside review, doc 50a C-4):** the last three rows were missing from Draft 0.1.
  The wage is the one that matters here, because it is paid to the port's authority.
- **Selling into a bid** (`+market/sell`) pays no listing fee, cut or levy; only the Co-op's share
  applies. A private ask pays the listing fee and the cut like any ask; a private bid sold into
  pays nothing (doc 50a B-4).
- **There is no tax, tariff, toll or duty on ordinary cargo**, and no passage, clearance or docking
  fee. "Clearance" in the voyage code is a launch gate.
- **"Republic fees" and "Co-op rates" are words in doc 11 and in room text.** No code or content
  defines a fee that differs by who holds a place. The Co-op's real terms are a stake of 25,000 cr,
  the share above and a buyout of 90,000 cr, all in the lane's `programme` block.
- **A port's `authority`** is one of its organizations, named in the lane pack: the Freeport
  Registry at Ceres, Stickney Yards at Stickney, and one each at Luna, Deimos and Titan. It is
  baked into the pack's version and digest. Nothing changes it while the server runs.
  - It has eight consumers: the venue's fees, the berth, the locker, crew wages, the captive
    handover, the release fee, the clerk's release capability (granted once a world) and a
    validation rule. Wage obligations re-point when the payee changes; berth and locker
    obligations never compare the payee (`port/berth.rs:112, 131`; doc 50a A-10d).
- **Terms that already move at runtime**, with no pack bump: the NPC price factor (5% an IC week
  within 50 to 200%, EX.8), the throttle, the reach premium, emergency bids, and a facility
  owner's customers and margin (PL.9). The last is the one case of runtime state laid over lane
  content.
- **What is pinned and what is live.** A works fee is pinned at booking. The Co-op's share and
  buyout are frozen on the enrollment. Haul pay is escrowed at posting. Venue fee rates are read
  live.
- **A fee in the lane pack costs** a version bump (`LANE_VERSION` is 41), a digest re-pin and a
  strip step in the kept-world tests. A fee as a Rust constant costs a code change.
- **No surface says why.** No quote itemizes the cut, the levy or the berth, and nothing names the
  reach premium, the throttle or the stock factor as a cause.

### 3.4 Operations, faction money, jobs and stockpiles

- **`mush-campaign`** holds declared operations (`@op/declare|terms|activate|join|withdraw|show|
  resolve|list`).
  - Anyone may declare, within a cap per issuer. The bond is a money hold on the issuer's own
    account; the crate never mints.
  - Notice runs from activation and must be 28 to 84 IC days (`record.rs:46-50`). It reaches
    eligible defenders at login and by IC mail.
  - Objectives are `WindowElapsed`, `Muster{minimum}` or `CanonRecorded{kind, source}`.
    Consequences are `Front{region, control}` or `Outcome{contract, row}`.
  - It ends in one canon record, `campaign.operation.resolved` or `.abandoned`. A faction clock
    that staff bound to that kind then completes and flips control. The campaign crate writes no
    control record itself and has no `mush-faction` edge by design.
  - It declares a list of legal-target classes, which nothing reads.
  - **Correction (outside review, doc 50a A-2):** five facts Draft 0.1 missed.
    - **The tick resolves a running operation itself**, every 15 seconds: "success" when the
      objective is met and "failure" when the window closes unmet (`tick.rs:291-355`). A
      `CanonRecorded` objective is read at every fire with no time test, so it succeeds the moment
      the record exists, mid-window, and the success is the declarer's.
    - **The resolution does not say who won** to a watcher. Both outcomes commit one canon kind
      with the outcome in the record's detail, and the event an observer receives carries no
      detail (`settle.rs:239-258`; `mush-core/src/module.rs:941-960`).
    - **Only an object principal** can declare, be a side or be a defender, and only through the
      `@op/*` commands. The crate exports no function to declare or activate. An organization
      named as a defender is notified of nothing (`notice.rs:108-111`).
    - **The bond is released on success and on failure.** Only a staff outcome forfeits it
      (`settle.rs:497-499`).
    - **Neither crate may take the other as a dependency.** TacSim is pinned without a campaign
      edge and the campaign crate without a faction edge. `mush-composition` already joins them by
      injected builders (`mush-composition/src/lib.rs:428-437`), and that is the seam a new link
      would use.
- **No TacSim source calls it.** The one seam a domain has is to commit canon, which the campaign
  tick reads. That seam is one-way: it can meet an objective, and it cannot declare an operation,
  read one or learn its outcome.
- **A faction is not a principal.** Its money is its backing organization's accounts.
- **The nearest precedent for a budget with a goal is House procurement**: a compiled tick every
  five seconds acts as the House's agent, with a monthly cap counted from its own ledger rows
  (`procurement/mod.rs`). Its goal is fixed (buy mind licenses).
- **Jobs.** `+market/jobs` lists retrieval, passage, survey, crew, haul and escort.
  - A haul or escort post takes no organization. Only `accept` takes `as=<org>`.
  - A haul runs 28 IC days to find a taker and a 28-day term. Its delivery fact is a derived
    inventory marker only the delivery leg can write. On a lapse the fee is refunded, the bond
    goes to the shipper and the cargo returns at the hull's next port.
  - The escort fact, `tacsim.escort.kept`, is written by the host: near the hull for 90% of the
    journey. It fails on a wreck, a capture, or the escort leaving.
  - An escort post names one hull the poster holds (by helm, title, rig lease or seat), docked for
    a passage, and is paid from the poster's own account (`services/escort.rs:721-806`). So an
    organization cannot post one, and nobody can before the hauler's hull is known (doc 50a A-8).
  - A retrieval is the owner paying to have their own hull brought in. It cannot be posted on a
    hull the poster does not hold title to (`mush-economy/src/contract_live.rs:6772-6790`; doc 50a
    A-7).
- **Doc 48's EX.10**, not built, promises an NPC hauling house whose real hulls run between ports
  when a price gap pays, and can be intercepted. The one NPC house that flies today, Tycho
  Mutual's retrieval arm, takes the long way round because "an NPC house never answers" the
  picket's customs bubble (`port/house_arm.rs:128-132`; doc 50a B-3c).
- **There is no stockpile, quota or requisition.** Goods sit in vault pools. The nearest ideas are
  the hull pool's floor, the intake drip's cap and EX.8's stock target.
- **Losses** leave their own records: a customs ruling, a hull's end state in space (operational,
  immobile, disabled, wreck), a wreck on a site, a capture. None reaches an operation unless it
  also commits canon.
- **In a space bubble every carrier is friendly to every other** (`space/tick.rs:5272-5277`).
  One player's ship cannot fire on another's. Doc 47 left this as an open design call.
  - **Correction (outside review, doc 50a C-5):** Draft 0.1 also cited `:5894`, which is test
    code.
  - A bubble holds a hull while any operational hostile hull remains in it
    (`space/tick.rs:521-524`). There is no verb to break off, no distance or time at which a
    bubble lets go, and no surrender in space (doc 50a A-9).
- **Opposition that needs no staff:** the picket and the interdictor, the raid director on a site,
  the compiled NPC sortie leader, and House hulls that staff seed.

### 3.5 Isidis

- **In the setting** (doc 11 §5, line 241) it is the Mars ground theater: "Landing field, tank
  farm, tariff-free market, Factor's office. The ground op's objective is *administration*: whoever
  holds it decides whether Belt cargo lands untaxed." De jure it is contested between the Republic
  and the Freeport; de facto Meridiani holds it for the Freeport; covertly the Tether; economically
  Occator and Ceres Clearing.
- **Its fronts** (doc 11 §6):
  - The Concession Review (Republic), six segments from two, advances on Assembly votes (a
    scheduled tick, one an IC month), incidents at Isidis, and a seized Freeport-flagged hauler.
    Filled, "the Republic **declares an operation** (T6) to assume administration".
  - Hold the Concession (Freeport), six from two, advances when Meridiani reinforcements land,
    "Occator stockpiles reach threshold", and a hauler runs the pickets and arrives at Isidis.
- **Who hires** (doc 11 §8): the Freeport side for "hauling Ceres → Kirkwood → Isidis, escort
  through the pickets"; the Republic for "customs escort, salvage of interdicted hulls".
- **In the game** it is prose: four mentions in cast and atlas text. No port, site, region,
  organization or clock.
- **The proving story** (doc 10 §11) ends: "deploy to Mars → complete a ground operation
  that changes control or access → salvage, repair, pay, repeat."

### 3.6 Offenses, evidence, penalties and standing

**Where the code decides an act was unlawful:**

| Where | The offense | The proof | What follows |
|---|---|---|---|
| The Kirkwood picket | unlicensed strategic ore; running the picket | the picket's own inspection | a fine or a seizure; a hostile corvette |
| A site's evidence log | pickup, recovery, scrap, vacated claim, capture | none: "evidence, never an enforcement" | nothing |
| Port security (RS.7) | a proven salvage row | a vacated contest, or a witness report backed by the witness's own record | a fine at docking at Ceres or Stickney, paid at once only in full; otherwise a debt with 28 IC days of grace, then the hull quarantined |
| The bay | a part from a vacated claim | the vacating contest | the part is flagged; no path strips under a claim yet, and no owner's verb clears one |
| The field | fire between parties; capture | fire receipts counted from the field's black box | standing lost; friendly fire is voidable by the leader, a capture's loss is not |
| The Registry (minds) | an ungoverned core, a grey die | `@registry/inspect` gathers the evidence | `@registry/fine`, `/revoke`, `/seize` and `/forfeit`: a sanction, a 90 IC day bar, the core impounded in place |

- **Correction (outside review, doc 50a C-2, C-6):** Draft 0.1's rows for port security, the bay,
  the field and the Registry were wrong in these ways.
  - Port security gives 28 IC days of grace before any quarantine, and takes a fine in full or not
    at all (`surface/security.rs:328-353, 462-469`).
  - A stripped part cannot be cleared by its owner: that path is dormant in TacSim
    (`port/bay_work/strip.rs:261-263`).
  - The field counts all fire between parties, and a capture's standing loss (−5) is "not
    voidable" (`field/crew.rs:1014-1017`). A capture carries no fine.
  - `@registry/inspect` is evidence only. The sanctions are separate staff verbs.
- **The witness's record is weaker than Draft 0.1 said.** Custody evidence has no time test at all.
  The other kind must have been learned within one IC day after the act, is one row a principal a
  site, and is overwritten by a return visit and removed when the site's list fills
  (`security.rs:588-602`, `site_record.rs:798-830`).
- **A past-due debt also closes two desks.** `+coop/enroll` and `+tv/lease` refuse a principal
  with any debt past its grace (`retrieval::refused_at_desk`).
- **Port security's ports are a constant**, `SECURITY_VENUES = ["ceres", "stickney"]`, and its
  fines (500, 2,000 and 5,000 cr) are paid to the Freeport Registry at both
  (`surface/security.rs:40-46`).
  - The constant gates only the fine at docking. The quarantine is tested at every departure from
    any berth, a site's pad included (`port/voyage.rs:3456-3458`; doc 50a A-10c).
  - An organization's hull answers to nobody there: the list of who answers is empty for one
    (`port/owned.rs:470-481`, `security.rs:314`; doc 50a A-10b).
- **Legality.** `mush_market::Legality` is `Legal`, `Restricted` ("sold under a license") or
  `Forbidden`. Its comment: "Recorded, rendered and filtered on; never enforced by this crate —
  jurisdiction is a policy test at transaction time (10b §3.2), and the ruleset owns it." Every
  TacSim catalog row writes `legal`. The venue gate admits by actor, venue, endpoint and presence;
  it does not see the good.
  - **Correction (outside review, doc 50a A-3):** the field is on the catalog entry, one a good,
    and a listing carries none. It is read by a list filter and a label and by nothing else.
  - **The seam that does see the good exists.** `ProceedsPolicy::proceeds` is given the seller, the
    venue, the commodity, the quantity and the price, and may refuse with its own line
    (`mush-market/src/module.rs:324-336, 408-417`). It is asked when a player sells into a bid and
    when a personal lot ask is bought. `may_list` is asked when an ask is posted and sees no
    commodity. Neither is asked of a house's own listings.
- **A private sale's record.** A private offer is hidden and tells only its two parties
  (`record.rs:918-927`). The settled quote holds the parties, the good, the venue, the price, the
  quantity and a wall-clock time; it holds no IC time and nothing about the seller's papers. Tape
  rows are capped at 32 a pair and 1,024 in all, and the oldest are evicted. The buyer reads it
  with `+market/orders`; the seller has no verb for it (doc 50a B-5).
- **Evidence a party already holds:** a site's sight records, a custody witness pin, the field's
  black box, a customs ruling, a contract's fact conditions, an inventory marker, canon in three
  scopes (public, restricted, secret). Nothing records the sale of a restricted good, because no
  good is restricted.
- **Penalties short of force that exist:** a one-term debt seen and paid with `+debts`; a berth
  lien collected at the hull's next sale; a quarantine that refuses departure; a 90-day bar on a
  principal; a charter recalled; custody with a release fee that becomes a debt, so "no balance
  ever holds a character" (Q-42-7). There is no ban and no impound of a market lot.
- **Standing** is a fold over immutable, cause-stamped grants, from −100 to 100 in five bands. It
  persists and does not decay. `+standing` shows it with its history. Its only readers are an
  organization policy test and the NPC prompts.
  - The bands are hostile to −50, unfriendly from −49 to −10, neutral from −9 to 9, friendly from
    10 to 49 and trusted from 50. Standing starts at 0 (`mush-faction/src/reputation.rs:510-533`).
  - The store holds 16,384 receipts in each of its maps and never evicts one. Full, it refuses new
    grants (`reputation.rs:18, 116-118`; doc 50a B-1).

### 3.7 What a player can read

`+clocks` lists the clocks of factions the viewer may see, as title, count and faction.
`+territory` and `+faction` show holdings. `+chronicle` shows public canon, newest first.
`+market/jobs` and `+market/history` show jobs and house prices. `@op/list` and `@op/show` show an
operation through a hidden, notice or plan view. The pilot gets a `[PICKET]` line.

There is no mail, notice or board line when a clock moves or a front closes. A newcomer at a port
learns of a past event only from `+chronicle` and from prose.

## 4. The gaps, and where the asks meet earlier rulings

### 4.1 The gaps

- **Kirkwood.** Six things are missing between the full clock and a price: the region, its control
  record, the flip, a fee that depends on the holder, a quote that shows it, and a line that says
  why.
- **Isidis.** The place, two of its three fronts, a scheduled cause for a clock, an organization
  that can post a job, a stockpile, a rule for a lost cargo, any link from TacSim to an operation,
  and a tax for the outcome to switch.
- **Law.** A good that is restricted, a place the restriction is tested, one record of proven
  offenses across the three enforcers, jurisdiction that is not a constant, a reader for standing,
  and a page that tells a player what stands against them and how to clear it.

### 4.2 The rulings this must fit

- **Control** changes "only via front/clock resolution or staff action — never by fiat inside a
  scene, never by an LLM" (`scenes-factions-knowledge.md:454`). A clock advances only from a typed
  cause with a stored cause id. A completion "may produce one consequence bundle".
- **No omniscient fine.** Doc 27 Q-S4: enforcement needs acquired evidence. Doc 41 Q-41-24:
  "proven rows only". Q-41-25: the fine at docking, and the hull quarantined until it settles.
  **Correction (outside review, doc 50a C-2):** as built, a fine that cannot be paid in full
  stands whole as a debt with 28 IC days of grace, and the quarantine begins only after that.
- **The 28 IC days.** Any wait on a player's response is at least 28 IC days. Q-45-15 keeps the
  operation notice for "a stake that changes what a player or organisation holds (a front or
  control change)"; "Fights need no operation." Q-45-14: an attacker may start anything at any
  time. Q-45-22: windows do not wait for a login.
- **Closed families stay closed** (Q-46-3, Q-48-1): no runtime-interpreted protocol. A new charge
  is compiled code.
- **Economy shape.** D11: no universal exchange. Q-48-42 and Q-48-57: a haul's bond and declared
  value. Q-48-20: the NPC hauling house, because "a raider also needs something to raid".
- **Institutions are compiled arms** whose numbers a designer authors (doc 44, "The world's
  institutions"; Q-44-32 defers Rhai policy).
- **Fields are multi-party**, never two sides (doc 36).
- **News travels by relay** (doc 15): the corridor learns that something happened at once and what
  it looked like later. A letter is in character and must transit. **Correction (outside review,
  doc 50a C-7):** an official notice does not transit. `mush_mail::post_notice` delivers to each
  inbox at once, and that is the channel Q-45-17 ruled for an operation's notice.
- **At 24:1 an IC day is a real hour**, so a rate per IC day is felt per real hour (Q-48-15).

**Where the asks meet a ruling head on:**

1. **"Republic fees" reverses Q-48-15.** That ruling reads "No customs duty", and its proposed text
   "No customs duty yet; customs stays a fine." A duty is the only way the outcome reaches a
   price. Q-50-6 asks for the reversal in plain terms.
2. **"Tax-free" is the status quo.** With no duty anywhere, the Isidis prize changes nothing until
   one side's administration taxes. Q-50-29.
3. **A control change is an operation with notice.** Isidis fits: doc 11 has the Review's
   completion declare one. Kirkwood's flip comes from a clock alone. Q-50-11 asks whether a front's
   public clock is notice enough.
4. **Facility assaults are out of scope**, and the proving story wants a ground operation that
   changes control. Q-50-28 proposes that supply settles it.
5. **One player's ship cannot fire on another's.** Escort work and interdiction between players
   wait on doc 47's open call. Q-50-26.
6. **No balance holds a character** (Q-42-7), so a restriction binds a hull or a trading right,
   never a person's movement. Q-50-39.

### 4.3 Defects and stale notes found (for the fix thread, not this plan)

- **Canon after the front closes.** Each later picket incident still commits a record that reads as
  the front advancing. The clock is full and the front resolved.
- **Staff `@clock/advance` on the customs clock** runs `fire_consequence` and leaves the front
  unresolved: the two completion paths disagree.
- **A seizure destroys matter.** The ore is debited and credited to nobody. **Correction (outside
  review, doc 50a C-8):** doc 48's census does have a `seized` term (`port/census.rs:38-39`), and
  nothing feeds it: both callers pass 0, and the customs seizure records nothing.
- **A Republic corvette collects for the Freeport Registry.** It may be intended (the license is
  the Freeport's). Q-50-14 asks.
- **A salvage fine at Stickney is paid to the Freeport Registry**, by constant.
- **Doc 11 against the code:** the customs clock is four segments in the text and six in code; the
  front is the Republic's in the text and on the Registry's board in code.
- **Doc 11 line 254** still says the relay backbone "is not built". Doc 15's R1 to R10 are built.
- **GOD-owned NPCs act for each other** (48-cut-notes, EX.3), so the market refuses a trade between
  two of them as a self-trade. Any NPC arm in this plan needs it fixed, or an agent object of its
  own as procurement uses.

**Found by the outside review (doc 50a), live in the tree today:**

- **The picket cannot see a bulk good held as a lot** (A-4). `read_pool` adds lots only for a
  part. Ore bought as a personal lot sits in the hold unread. The hold's fill check undercounts
  the same way (`space/mining.rs:1301-1320`). Every unit here that measures a hold waits on this.
- **A seizure cannot take ore under a haul's hold** (A-5). When all the ore aboard is reserved and
  the principal cannot pay, the ruling fails three times, the crew is told "she is not cleared",
  and a new submission repeats it. The ways out are credit or a run.
- **A declared operation with a bond is paused "unfunded" before it is activated**, and activation
  does not clear the pause (A-2g). The operation never runs. Any campaign tick between
  `@op/terms bond=` and `@op/activate` triggers it.
- **A passage escort settles at the first port** the protected hull docks at, not at its stated
  destination, when the passage takes more than one hop (A-8b).
- **A passage escort fails as "left"** if the escort's hull is still docked one port fire, about
  five real seconds, after the protected hull's journey appears (A-8b).
- **An organization's hull answers to nobody at port security** (A-10b).
- **A past-due fine holds the hull at every port and site**, not only where port security answers
  (A-10c). The module does not say whether that is meant.
- **Burning past 35 km records the hull as its own pilot** and writes no customs ruling
  (`space/tick.rs:4549`; A-10e).
- **Registry mind fines do not show in `+debts`** and `+debts/pay` cannot pay them (A-10e).
- **A free-flight autopilot from Ceres' halo to Deimos owes no picket leg** (B-3b). It is a way
  past the picket for contraband today.
- **A witness's salvage record is overwritten by a return visit** to the site more than a day
  later, which destroys the evidence port security needs (C-6).

Two more are latent, and bite only when this plan is built: the control flip that leaves two
holders (A-1), and the berth and locker obligations that never re-point (A-10d).

## 5. The shape proposed

**One record of who holds what.** The lane pack seeds a region for each place the game uses, with
doc 11 §5's four holders. The faction engine keeps the record. TacSim reads it through one
function.

**One schedule turns a holder into terms.** For each region and each polity that may hold it, the
lane pack carries a short closed list of numbers: a transit duty, an import duty, which goods are
restricted and by what paper, how often a hold is audited, what each offense costs. Compiled code
reads the row for the region's de facto holder. There is no effect language.

**Fronts and operations change control. Terms and law follow.**
- Kirkwood: the customs clock completes through the engine's own path and flips the approach to
  the Republic de facto. The schedule's Republic row carries a transit duty. The picket assesses it,
  the filing shows it, and every such line says who holds the approach and since when.
- Isidis: the Concession Review fills on a scheduled tick and on seized haulers. A compiled arm
  declares the operation with the notice the rule requires. The Freeport side spends a real budget
  on haul jobs, escort jobs and a standing bid to fill the tank farm. At the window's close the
  stockpile decides. The outcome flips de facto administration, and the schedule's import duty at
  Isidis is zero under one holder and not under the other.
- Law: what is restricted at a port is the holder's law. The public board refuses an unlicensed
  seller. A private sale goes through and is an offense if an inspection or a witness's own record
  proves it. A proven offense is written once, to the polity that proved it, and binds at the
  ports that polity holds. A player reads one page that says what stands against them and what
  lifts it.

**What this is not.** It adds no assault, no bounty on a person, no appeal court, no knowledge
model and no new scripting surface.

**Correction (outside review, doc 50a).** The shape stands. Four of its sentences were firmer than
the code allows:
- "Completes through the engine's own path and flips the approach" needs the engine to move the
  prior holder out, which it does not do today (Q-50-50).
- "A compiled arm declares the operation" and "the stockpile decides" need the campaign engine to
  gain a way to be declared from code, organizations as sides, and an objective read at the
  window's close (Q-50-57).
- "The picket assesses it" needs the picket to see cargo held as a lot, and meets only the direct
  run (Q-50-53, Q-50-55).
- "The public board refuses an unlicensed seller" has to cover selling into a bid, and the buyer
  (Q-50-34, Q-50-65).

## 6. Owner survey

Every question has a proposed default, an alternative and a reason.

**Where each stands at Draft 0.4.** The owner ruled three times on 2026-11-19, and all 71
questions are ruled. Each heading says how.
- **Accepted as written** (fifteen): Q-50-3, -5, -11, -12, -13, -14, -41, -45, -46, -48 and -49
  by the first ruling; Q-50-8, -15, -16 and -17, which the outside review cited and left
  unchanged, by the second.
- **Accepted as corrected** (34): every other question from Q-50-1 to Q-50-47, by the second
  ruling. The original text is kept, and the correction follows it and governs.
- **Accepted as proposed** (22, the follow-ups of §6.6): seventeen by the second ruling; and
  Q-50-55, -57, -62, -65 and -67, which the lead had held for the owner's own call, by the
  third.

**What the third ruling settled in answers already ruled.** Each of these had waited on one of
the five.
- **Q-50-57:** `mush-campaign` holds the operation's frame, with the five additions. That
  settles the frame in Q-50-18, -19, -21, -28 and -30, the seam Q-50-26 reads, and the empty
  run of Q-50-47. The TacSim-only operation record is not built.
- **Q-50-62:** a hull may break off, so Q-50-26 is built for the first operation.
- **Q-50-65:** one papers test, a license and a registered claim. It lifts the restriction in
  Q-50-32. The board refuses an unlicensed buyer as well as an unlicensed seller (Q-50-34).
  An audit asks it of a hold, and holding without the papers is an offense (Q-50-36).
- **Q-50-55:** the duty of Q-50-6 reaches the direct run. The long way round stays free of a
  picket, the free-flight shortcut closes, and the outward run stays free.
- **Q-50-67:** the reporting buyer hands over the goods, in Q-50-43 and in the third story of
  Q-50-47.

### 6.1 The model

**Q-50-1 — Where control lives.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** the faction engine's regions and control records are the one record of who
  holds a place. TacSim reads them through one function, and a front's completion flips one through
  the engine's own `on_complete_control`. **(expensive to reverse)**
- **Correction (outside review, doc 50a A-1):** the engine's flip does not move the prior holder
  out, stamps the hold with wall-clock time, and offers no function that seeds control. The
  default stands on Q-50-50's change to the engine.
- **Alternative:** a TacSim record per port or lane node; or a terms overlay written by each front.
- **Why:** the engine was built for this and its rule is already ruled (control changes only by a
  clock or by staff). `+territory` shows it. Doc 11 §5 is written in its four kinds. A second
  record would be a twin of the first.

**Q-50-2 — The polities as factions, from content.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** at boot each lane organization's `faction` tag is bound to a faction of
  that name, made from content when it is missing. The bring-up stops making the Republic by hand.
  The Houses stay organizations under their polity.
- **Correction (outside review, doc 50a A-10a, C-3):** there are six tags, so six polities;
  `townships` is the sixth. The binding is a table TacSim keeps, from each lane organization to
  its polity, read from the tag. It is not the faction's backing-organization field: that holds
  one organization, and the Freeport Registry's own faction already uses it. A polity gets a
  backing organization only where it needs a treasury (Q-50-14).
- **Alternative:** keep the staff-made factions, and name them to TacSim in the instance config.
- **Why:** the tag has waited for "the faction cut" since lead ruling L16. A front on the
  Republic's board needs the Republic to exist on a fresh world with no staff step.

**Q-50-3 — Regions, from content.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** the lane pack gains a `regions` block: one row for each doc 11 §5 row the
  game uses, naming its places (port entries, lane nodes, sites) and its four holders at world
  start. The first cut seeds the Kirkwood approach, Ceres and Stickney. Isidis arrives with its own
  unit. One lane bump.
- **Alternative:** staff make regions with `@region` at bring-up.
- **Why:** control has to exist on a fresh world. Content is reviewed and pinned by digest.

**Q-50-4 — Which kind of control sets what.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** fees and enforcement follow **de facto** control. Licenses, permits and
  claims follow **de jure**. Covert and economic control are recorded and read by nothing yet.
  **(expensive to reverse)**
- **Correction (outside review, doc 50a D-6):** a region may have two de jure claimants. Doc 11 §5
  gives Isidis two, and its §9 question 2 on that is unruled. Q-50-52 says whose papers an
  enforcer then reads. "The de facto holder" is one holder only after Q-50-50.
- **Alternative:** one holder a region, with no kinds.
- **Why:** the Kirkwood row needs the split: the Freeport licenses, the Co-op runs the depot, the
  Republic contests. Customs Assertion's outcome is written as "de facto". The split is also what
  gives a smuggler an argument: licensed by one authority and taxed by another.

**Q-50-5 — How terms hang off control.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** a terms schedule in the lane pack. For each region and each polity that
  may hold it: a transit duty, an import duty with its list of goods, the restricted goods and the
  paper that lifts each, an audit rate, and a fine scale. A missing row means the terms as they are
  today. All numbers provisional.
- **Alternative A:** Rust constants first, with no lane bump, and content later.
- **Alternative B:** a typed list of effects carried on the clock (a fee change, a rate change).
- **Why:** a designer authors numbers and never behavior (doc 44, "The world's institutions"). One
  schedule serves both
  Kirkwood and Isidis, and law (Q-50-33). Effects on a clock would be a second place to look, and
  a step toward the interpreted engine Q-46-3 refused.

### 6.2 The Kirkwood cut

**Q-50-6 — What "Republic fees apply to the lane" is.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** a **transit duty**, assessed at the picket's inspection on every laden hull
  that submits while the Republic holds the approach de facto. It is a share of the cargo's
  reference value (provisional 2%, floor 50 cr), paid to the Republic's customs office (Q-50-14).
  An empty hull pays nothing. **This reverses Q-48-15's "No customs duty"** for places whose holder
  levies one.
- **Alternative A:** a flat toll a hull.
- **Alternative B:** no new charge; the Republic takes over the venue fees and the berth at
  Kirkwood Depot.
- **Why:** the outcome says fees, every inward hauler meets the picket, and the customs ruling is
  already where a charge lands. Reference value is the measure Q-48-57 uses for a haul's declared
  value. Alternative B changes who is paid and not what a hauler pays.
- **Correction (outside review, doc 50a A-4, B-3a):**
  - "Every inward hauler meets the picket" was wrong. Only the direct run does. The long way
    round by Luna and Deimos carries no picket (Q-50-55).
  - The picket's reading of the hold must count lots first (§4.3), or cargo held as a lot pays
    nothing.
  - What is assessed, and at what value, is Q-50-53. A pilot who cannot pay, or who is away, is
    Q-50-54.
- **With this ruled (2026-11-19):** Q-48-15's "No customs duty" is reversed for a place whose
  holder levies one. Doc 48 carries a note at Q-48-15.

**Q-50-7 — What "the Co-op's rates rise" is.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** new Co-op enrollments sign at a higher share while the Republic holds the
  approach (provisional 300‰ against 250‰). Standing enrollments keep the terms frozen at signing.
- **Alternative A:** a surcharge on every chartered sale while the hold stands.
- **Alternative B:** leave it as prose in the first cut; the duty is the price change.
- **Why:** the share and the buyout are frozen on the enrollment, which is the charter's promise.
  The duty already reaches every hauler, chartered or not.
- **Correction (outside review, doc 50a B-6):** this reaches fewer captains than it reads.
  - An enrollment is one a gateway account and frozen at signing, and a captain who bought out
    their Dray never enrolls again. The new share is met by a new account, and by a captain who
    recharters after a dormancy or a release.
  - The share is one number in the lane's programme block today. The unit reads it from the
    schedule at signing, and the two lines of copy that say "a quarter" become templates.
  - The duty reaches every hauler on the direct run, not every hauler.

**Q-50-8 — Which quote shows it.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** three places. Where a passage through Kirkwood is planned or filed, a line
  with the duty estimated for the cargo aboard. The picket's ruling line. The job board, where a
  haul routed through Kirkwood shows the duty its hauler will meet.
- **Alternative:** the ruling line only.
- **Why:** the owner's test is the same run before and after, and a hauler decides before launch.
  §8 notes the survey did not find where a passage is priced before launch.
- **Outside review (doc 50a D-1): unchanged.** The screen exists. `+ship/plan` is the dry
  run before launch: it shows the waypoints, the burn and the ETA, and no money. The duty estimate
  goes there and on the filing.

**Q-50-9 — The reason line.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** every charge that control sets carries a clause naming the holder and the
  IC date its hold began, within 120 columns. `+territory` and `+chronicle` show the same record.
- **Correction (outside review, doc 50a A-1):** the control record holds wall-clock time, and
  nothing shows it. The clause needs the IC date Q-50-50 adds.
- **Alternative:** a `+terms <place>` page, and no clause on the charge.
- **Why:** no surface in the game says why a price is what it is. The owner asked that a hauler
  "be able to find out why".

**Q-50-10 — A passage under way when control flips.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** terms are pinned when a passage is filed. A hull that launched before the
  flip clears under the old terms.
- **Correction (outside review, doc 50a B-3b):** a passage engaged from free flight by
  `+helm/autopilot` files nothing today. Terms are pinned at that engagement as well.
- **Alternative:** the picket reads the terms live.
- **Why:** at 24:1 a passage spans real hours. A player who launched and logged off made no
  decision about the new price. A works fee is pinned at booking for the same reason (Q-48-53).

**Q-50-11 — When the new terms begin.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** at the clock's completion, for passages filed after it. No operation and no
  waiting period: a front's public clock, visible from the first day, is its notice.
- **Alternative:** the terms begin 28 IC days after completion.
- **Why:** Q-45-15 keeps the operation notice for a change in what a player or organization holds.
  This changes a price for everyone, as EX.8's weekly step does without notice. One caution: the
  Co-op, an NPC organization, does lose its de facto hold.

**Q-50-12 — What advances the clock, and its size.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** as built: a fine, a seizure or a run each advance it by one, and a clean
  inspection does not. Six segments, one filled, provisional. The front is seeded at world start so
  `+clocks` shows it from the first day.
- **Alternative:** doc 11's four segments; or a clean, paid inspection also counts.
- **Why:** the code and doc 11 disagree on both points. With smuggling as the only cause, a hauler
  who runs unlicensed ore raises every hauler's costs, which is the conflict of interest §6.4
  wants on this route.

**Q-50-13 — Whose front it is.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** the Republic's board, as doc 11 has it. Today it is on the Freeport
  Registry's.
- **Alternative:** leave it where it is.
- **Why:** the code opens it on the license issuer's faction because that organization was at hand.
  The outcome is the Republic's gain. Needs Q-50-2.

**Q-50-14 — Who is paid.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** before the flip, as built: the contraband fine goes to the Freeport
  Registry, whose license it enforces. After it, fines and duty on the approach go to a Republic
  customs office, a new organization row in the lane (provisional capital).
- **Alternative:** Stickney Yards as the Republic's payee, with no new organization.
- **Why:** enforcement follows de facto control (Q-50-4). The owner may also want to rule on
  today's oddity, a Republic corvette collecting for the Freeport.

**Q-50-15 — Whether the picket's own findings change.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** no. Under the Republic's hold the picket still fines when the pilot can pay
  and seizes when they cannot. The duty is the one change.
- **Alternative:** under the Republic's hold unlicensed ore is always seized, which is what the
  consequence text says.
- **Why:** one change at a time. The first cut's test is one price measured before and after.
- **Outside review (doc 50a B-2): unchanged.** The review cites this only to show that a
  lawful supply hull is never seized. That is Q-50-20's problem, and it is answered there.

**Q-50-16 — Whether it can be pushed back.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** not in the first cut. The hold stands until a later unit gives the Freeport
  side a clock that flips it back, which is the engine's own model of opposition, or staff act.
- **Alternative:** an opposing clock on the Freeport's board now, advanced by licensed strategic
  ore clearing the picket.
- **Why:** the owner asked for a small first cut. Worlds are wiped between steps (Q-38-9). The
  causes of a counter-clock belong with the campaign work of §6.3.
- **Outside review (doc 50a A-1): unchanged.** A later push-back needs the call of Q-50-51
  that reopens a front and sets a clock, which the engine lacks today.

### 6.3 The Isidis operation

**Q-50-17 — What Isidis is, in play.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** a port entry of the lane: the landing field, the tank farm (a vault with
  stock), the tariff-free market (a venue) and the Factor's office (its authority). It is a region
  and a neighborhood pack pair, like the moon ports of doc 19 cut A.8. How a Dray's cargo reaches
  the surface is fiction for the owner to settle.
- **Alternative A:** no new place. Cargo consigned to Isidis clears at Stickney's Customs Hall,
  where doc 11 §4.18 already has Belt cargo clear, and the Concession is a customs status on
  Stickney's venue.
- **Alternative B:** a surface site, with a pad and blocks, for a field.
- **Why:** the milestone is a newcomer who arrives and reads the terms. A port is content, not
  engine. Alternative A is the cheap path if the atlas work is unwelcome now.
- **Outside review (doc 50a D-7): unchanged.** The review supports the default against
  alternative B. No Dray can land on Mars (131‰ laden against a floor of 1,200‰), so a surface
  site would refuse every hauler. Isidis's routes are Q-50-58.

**Q-50-18 — What the operation is, in the engine.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** both engines, each used as built. A `mush-campaign` operation is the frame:
  declared, bond held, notice, window, sides, stakes, one resolution record. The two fronts' clocks
  are the visible progress. Resolution commits the operation's canon record; a clock watching it
  completes and flips the control record. TacSim gains no campaign edge: it commits canon, and the
  campaign tick reads it.
- **Correction (outside review, doc 50a A-2):** "each used as built" was wrong, and so was
  "it commits canon, and the campaign tick reads it". As built the campaign engine would resolve
  this operation in mid-window and for the wrong side, would not tell a clock who won, cannot take
  an organization as a side or a defender, and cannot be declared from code. The frame stands only
  with the additions of Q-50-57, which also asks whether to keep the engine at all.
- **Alternative:** fronts and clocks only, with no operation.
- **Why:** Q-45-15 already rules that a change of control is an operation with notice. Doc 11 has
  the Review's completion "declare an operation". The engine was built for this and nothing has
  used it.

**Q-50-19 — Who declares, and when.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** a compiled arm on the port tick acts for the Republic. When the Concession
  Review fills it declares the operation from the Republic's treasury, bond held, naming the
  Freeport and Meridiani as defenders, with 28 IC days' notice and a 28-day window. No player or
  staff step. At 24:1 that is about 56 real hours end to end.
- **Correction (outside review, doc 50a A-2f):** no code path can declare today, and an
  organization named as a defender is told nothing. Both are in Q-50-57. The arm sets the 28-day
  window itself; `@op/declare` defaults it to 24 hours.
- **Alternative:** staff declare; or a player officer of a Republic House may.
- **Why:** the faction design's own test is "no faction-head bottleneck". Institutions are
  compiled arms today.

**Q-50-20 — What moves the Review before that.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** the two causes of doc 11 the code can supply: a seized Freeport-flagged
  hauler at Kirkwood, which the picket already writes as canon, and a scheduled tick of one an IC
  month (about 30 real hours). The scheduled tick is a new typed cause, deferred since M25. The
  clock starts at two of six.
- **Correction (outside review, doc 50a B-2):** as written the first cause could not fire. No hull
  carries a flag, and the picket seizes only unlicensed strategic ore from a pilot who cannot pay,
  which a lawful supply hull never carries. Q-50-61 defines the flag. Under it the cause is a
  seizure ruling on a Freeport-flagged hull, in practice a Co-op Dray. Both causes reach the clock
  as typed owner calls (Q-50-51), not through canon.
- **Alternative:** seizures only.
- **Why:** doc 11 §6 says these clocks are "what move when nobody is logged in".

**Q-50-21 — The stockpile objective.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** a named vault at Isidis (the tank farm), a list of goods with target
  tonnages (provisional: propellant and provisions) and a deadline, the window's close. Progress is
  the vault's level, read on the port tick. "Threshold reached" is committed once as canon, which
  the operation's objective reads.
- **Correction (outside review, doc 50a A-2a):** an objective that watched for that record would
  end the operation the moment the farm filled, as the declarer's success. The level is read
  once, at the window's close (Q-50-57). The canon line stays as news for `+chronicle` and decides
  nothing. The goods are Q-50-58's, and what a repeat starts from is Q-50-60.
- **Alternative:** count deliveries as events, each haul's marker advancing a clock.
- **Why:** doc 11 words it "stockpiles reach threshold". A level can be fed by a haul job, by a
  sale into a standing bid or by the faction's own hull, so a player who took no job still counts.
  A level is safe across a restart. Events double count on a retry unless each carries a cause id.

**Q-50-22 — The budget, and who spends it.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** the Freeport side's money is its backing organization's treasury (Occator,
  for the tank farm). A compiled arm on the procurement pattern, with a monthly cap counted from
  its own ledger rows and an agent object of its own, posts haul jobs from Occator's stock at Ceres
  to the tank farm, escort jobs for them, and a standing bid at Isidis. When the cap or the
  treasury runs out, posting stops. No money is minted for it.
- **Correction (outside review, doc 50a A-8a):** the arm cannot post an escort for a haul today,
  because a post names a hull the poster holds. Q-50-63.
- **Alternative:** staff post; or the faction posts a bid and no jobs.
- **Why:** procurement is the working precedent of an organization spending toward a goal with no
  player. A real budget makes interdiction a way to bleed it.

**Q-50-23 — Posting as an organization.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** haul and escort posts gain `as=<org>` for a member holding a new
  capability, paid from the organization's account and shipped vault to vault. The NPC arm uses the
  same path.
- **Correction (outside review, doc 50a A-8a):** for a haul this is as proposed. An escort needs
  more than `as=<org>`: it must be postable for a haul whose hull is not yet known (Q-50-63).
- **Alternative:** only the NPC arm gets an organization path.
- **Why:** today only a character standing in a port can post, from their own locker to their own.
  A player House will want this the day after. It needs §4.3's NPC defect fixed or worked around.

**Q-50-24 — If no player takes the job.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** after a provisional 7 IC days an untaken haul is carried by doc 48's NPC
  hauling house (EX.10) in a real hull that can be intercepted. The operation resolves with nobody
  connected.
- **Alternative:** an untaken job lapses, and the faction is simply short.
- **Why:** the faction design's test: delete every player, and the conflict must still advance.
  It depends on EX.10 landing.
- **Correction (outside review, doc 50a B-3c):** it depends on more than that. No NPC hull
  answers the picket today, and the one NPC house that flies avoids it for that reason. EX.10's
  hulls must answer it by rule (Q-50-56) before one can carry a haul on the direct run.

**Q-50-25 — What a loss does.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** cargo seized, wrecked or captured on the way never reaches the stockpile.
  It costs the hauler the bond, as today. When the hull flew for a side of the operation it also
  commits a canon record that advances the other side's clock by one. The haul gains the handling
  it lacks: on a wreck the reserved cargo becomes part of the wreck, and on a seizure the job fails
  at once.
- **Correction (outside review, doc 50a A-5):** a seizure cannot take cargo under a haul's hold
  today, and when all the cargo is held the ruling never settles (§4.3). The unit makes the ruling
  release the hold on what it takes and fail the job in the same transaction. "Flew for a side"
  is Q-50-61, and the record that advances the other side's clock is a typed call (Q-50-51).
- **Alternative:** money only.
- **Why:** doc 11 already names a seized Freeport-flagged hauler as a cause for the Review. The
  owner asked that "deliveries or losses affect the declared operation".

**Q-50-26 — Whether players may fight over a hauler in space.**
**Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** inside a declared operation's window, hulls flying for its named sides are
  legal targets to each other in a bubble, read from the operation's legal-target list. Outside it
  today's rule stands: every carrier is friendly to every other. This answers doc 47's open call
  for this one case. **(expensive to reverse)**
- **Correction (outside review, doc 50a A-9, A-2h):** three things this needs.
  - **A way out.** Two hostile hulls hold each other in a bubble until one is disabled. Q-50-62
    adds one, and this default is not built without it.
  - **A rule for whose side a hull is on** (Q-50-61).
  - **A seam.** TacSim cannot read an operation's window or its sides today (Q-50-57).
- **Alternative A:** no change. Only the picket, the interdictor and raiders threaten a hauler.
- **Alternative B:** any carrier may fire on any other, the act recorded as evidence (§6.4).
- **Why:** an escort has nothing to guard against but NPCs today. Limiting it to a declared window
  leaves doc 45's absent-player rules as they are everywhere else.

**Q-50-27 — Work for players on the Republic's side.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** in the first operation the Republic's arm posts a standing bid at Stickney
  for the same goods at a premium, and retrieval jobs on interdicted hulls. Both use job kinds that
  exist.
- **Correction (outside review, doc 50a A-7):** only the bid can be posted. A retrieval is the
  owner paying to recover their own hull, so the Republic cannot post one on a hull it does not
  own. The corrected default is the standing bid, and escort jobs for the Republic's own supply
  hulls once Q-50-63 exists. Salvage of interdicted hulls waits for a contract of its own, in a
  later unit.
- **Alternative:** nothing for the Republic's side in the first operation.
- **Why:** two buyers for one cargo is a choice a trader can feel with no new job kind: every tonne
  sold to the Republic is one not landed at Isidis. A campaign with one side's work is a quest.

**Q-50-28 — How it is decided.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** at the window's close. If the threshold record exists, the Concession is
  held and the Freeport keeps de facto administration. If it does not, the Republic assumes it.
  The declaring arm resolves; staff do not. No fight is required and none is forbidden.
- **Correction (outside review, doc 50a A-2a):** the campaign tick resolves an operation by itself
  at the window's close, so "the declaring arm resolves" is not a step anyone takes. The rule
  stands as the objective's meaning: the stockpile's level is read at close, and at its threshold
  the defenders hold (Q-50-57).
- **Alternative:** a ground engagement at Isidis decides it, and the stockpile only weights it.
- **Why:** facility assaults and territorial warfare are out of scope by ruling. Supply is what
  docs 48 and 49 just built. A declared operation over a ground theater, settled by what was
  landed there, is the smallest honest reading of "a ground operation that changes control or
  access". The owner may prefer the alternative once assaults are in scope.

**Q-50-29 — What winning changes.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** the de facto control record for Isidis, and through the schedule the
  import duty there: zero while the Freeport administers it, and a provisional 5% of reference
  value under the Republic, on a list of goods in the schedule, paid to Republic customs. The
  venue fees' and the berth's payee follows the holder as well.
- **Correction (outside review, doc 50a A-10d):** the last sentence is withdrawn for the first
  operation. The port's authority has eight consumers, and two of them never re-point; Q-50-64
  proposes that only the duty follows the holder. When the duty is taken, and from whom, is
  Q-50-59.
- **Alternative:** the de jure record changes too, and with it who issues licenses there.
- **Why:** "whoever holds it decides whether Belt cargo lands untaxed." Untaxed is the only state
  today, so the prize needs the tax. The duty names goods and not origin because the survey found
  nothing on a lot that records where it came from (§8).

**Q-50-30 — Following it.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** one player page, `+operation`, on one screen: who declared against whom;
  the stake in plain words, as what each outcome changes in the terms; the notice and the window
  in IC and real time; both clocks; the stockpile's level against its target; the jobs open for
  it. Past operations stay listed with their result. The job board tags an operation's jobs. A
  queued notice and IC mail go out at the declaration and at the resolution to every player with a
  hull on the lane (doc 45 Q-45-17's channel).
- **Correction (outside review, doc 50a A-2h, C-7):** the page joins an operation, two clocks, a
  vault and the job board, and TacSim reads the first only through the seam of Q-50-57. The
  notice is an official notice, delivered at once, as Q-45-17 ruled. It is not a letter in
  transit.
- **Alternative:** `+clocks`, `+chronicle` and the job board as they are.
- **Why:** the owner's words are "discover, influence, and follow". `@op/show` is shaped for staff.
  Nothing today joins the pieces.

**Q-50-31 — Whether it repeats.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** yes, by the same shape. A held Concession sets the Review back to two of
  six, and it climbs again. An assumed one opens a Freeport front to win it back, sides swapped.
  Only the first operation is built first.
- **Correction (outside review, doc 50a A-1):** nothing can set a clock back or reopen a front
  today (Q-50-51). And without a drain the Freeport would begin the second operation already past
  its threshold (Q-50-60).
- **Alternative:** one operation a world.
- **Why:** worlds are wiped today, but the shape should cycle when they are not.

### 6.4 Law: jurisdiction, evidence, penalties and standing

**Q-50-32 — The first restricted good, and its port.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** strategic ore at Ceres. Its sale on the Freeport Exchange is restricted:
  sold under the Registry's prospecting license or a registered claim, the two papers the picket
  already reads. It stays lawful to sell at Stickney and Luna.
- **Correction (outside review, doc 50a C-1):** "or" was wrong. The picket asks for both papers,
  and so does this (Q-50-65). The house bids confirm the loop: 600 cr at Ceres, 700 at Stickney
  and 650 at Luna.
- **Alternative:** space munitions at Stickney, under Republic law; or a mind core, which its own
  closed families already police.
- **Why:** the rule, the papers and the check exist. It closes a loop with §6.2: ore that cannot be
  sold on the Ceres board is run inward past the Kirkwood picket to Stickney, where the house bid
  is also higher (700 against 600 in the lane pack), and each run caught there advances Customs
  Assertion.

**Q-50-33 — Where legality lives.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** in the terms schedule, per polity: a list of goods, each restricted or
  forbidden, with the paper that lifts a restriction. A venue's rule for a good is the law of its
  region's de facto holder. Catalog rows read it in place of the hard-coded `legal`.
- **Correction (outside review, doc 50a A-3):** a catalog row is one for a good everywhere, so it
  cannot carry a venue's law. The law is asked for a venue and a good at the moment of a trade,
  through the market's proceeds policy, which already sees both and may refuse. `may_list` gains
  the commodity, so a listing can be refused when it is posted. The catalog's field stays
  `legal`.
- **Alternative:** per venue, on the catalog rows.
- **Why:** law follows control (Q-50-4), so when Kirkwood or Isidis changes hands, what is
  restricted there changes with it and no second table is kept. The market's own comment gives the
  test to the ruleset.

**Q-50-34 — Where it is enforced.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** at both ends, differently. The public board refuses an unlicensed seller
  of a restricted good: the exchange checks papers, so the venue gate learns the good. A private
  offer between players goes through, and is an offense if it comes to light. A forbidden good
  could not be listed at all; none is yet.
- **Correction (outside review, doc 50a B-4):** "the venue gate learns the good" named the wrong
  seam, and the default covered one of the two public ways to sell. A player also sells by
  `+market/sell` into a public bid, and the houses' bids are where most ore goes. The board
  refuses on both paths, through the proceeds policy. Q-50-65 adds the buyer's side, and Q-50-71
  the fees a private sale pays.
- **Alternative A:** refuse everywhere. That is a rule with no crime in it.
- **Alternative B:** refuse nothing, and act only after the fact.
- **Why:** this gives the smuggler a reason to find a private buyer, the buyer a discount to ask
  for, and the licensed trader a clean path. An exchange that sold contraband openly would make
  the license worthless.

**Q-50-35 — What establishes an offense.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** port security's standard, generalized. An authority's own inspection: the
  picket's manifest check, or a port audit (Q-50-36). A witness report backed by the witness's own
  record no more than one IC day old: a party to the trade, or one present at the venue. An
  adjudicated contest. A bare log never fines.
- **Correction (outside review, doc 50a B-5, C-6):** "or one present at the venue" is withdrawn.
  A private offer is hidden and tells only its two parties, so a bystander holds no record. A
  party's record is the receipt of Q-50-66. Port security's own standard is also looser than
  stated: its custody evidence has no time test today (§3.6).
- **Alternative:** the trade tape proves every sale.
- **Why:** Q-S4 forbids the automatic omniscient fine, and Q-41-24 says "proven rows only".

**Q-50-36 — The port audit.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** at docking in a port whose law restricts a good, the hold is audited with a
  chance from the schedule, moved by the pilot's standing there: provisional one in four at
  neutral, always in the hostile band, never when trusted. It is a seeded roll, so a story can pin
  it.
- **Correction (outside review, doc 50a A-6a, D-4, A-4, B-1):**
  - The audit runs on the port tick after the hull docks. The arrival hook cannot read a hold.
  - The port tick has no seeded roll. The audit's is a keyed hash of the hull and its docking, on
    the pattern the survey deposits use, so a story can still pin it.
  - It reads lots as well as the pool (§4.3).
  - The chance for each band is Q-50-69's.
- **Alternative A:** every docking is audited.
- **Alternative B:** no port audit; only the picket and witnesses.
- **Why:** "if caught" needs a chance, or smuggling is either impossible or free. It gives
  standing its first mechanical reader.

**Q-50-37 — One record of proven offenses.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** each domain keeps its evidence where it is: the site log, the customs
  rulings and the Registry's inspections stay as they are. A proven offense is written through one
  function into a record per actor and per polity, which is port security's own record with a
  jurisdiction and a closed list of kinds added. Penalties and the player's page read that record.
  The Roll's sanctions are listed on the page and not moved. **(expensive to reverse)** in its key,
  actor by polity.
- **Correction (outside review, doc 50a A-10b, A-10e):** "actor" needs defining before the key is
  fixed: a character, an organization acting through a member, or the gateway account a license
  is kept by (Q-50-68). The Registry's seizure rows name no owner, so the page can list a live
  impound and not a past forfeiture (Q-50-70).
- **Alternative:** one ledger that replaces the domain records.
- **Why:** closed families stay closed. Port security's record is already half of this. Three
  enforcers need one reader.

**Q-50-38 — Jurisdiction.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** an offense is proven by one polity and binds only where that polity holds
  de facto. Other ports ignore it. Port security's constant list of ports becomes "the ports held
  by the polity that proved it". A fine is paid to the enforcing authority: the port's `authority`,
  or the customs office on an approach.
- **Correction (outside review, doc 50a A-10c):** today a past-due fine holds the hull at every
  port and site, whatever the constant says (§4.3). This default narrows that to the proving
  polity's ports. It is a change in behavior, not only a new list.
- **Alternative:** one record for the whole corridor.
- **Why:** it gives crime a geography: wanted at Stickney and clean at Ceres is a route choice. It
  follows Q-50-4. It also ends a Stickney fine being paid to the Freeport Registry.

**Q-50-39 — The penalties.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default,** in order:
  - **A fine** by offense kind, scaled by tonnage for goods, from the schedule, posted from the
    actor's account. Unpaid, it is an obligation with 28 IC days' grace, then a quarantine of the
    docking hull at that polity's ports until it settles, as Q-41-25 built.
  - **A standing loss** with that polity for the offenses the schedule marks. Provisional: none
    for a fine paid at once, −3 for a sale proven by a witness, −5 for running the picket.
  - **A trading restriction** while standing there is below neutral or a fine is unpaid: the
    polity's venues refuse the actor's listings of restricted goods, and its Registry issues the
    actor no new license.
  - Nothing holds a character (Q-42-7).
- **Correction (outside review, doc 50a B-1, C-2):**
  - The sizes did not reach the bands. Neutral runs from −9 to 9, so −3 and −5 left an offender
    neutral. Q-50-69 resets the sizes and defines "below neutral".
  - "A quarantine of the docking hull" is narrower than what is built, which holds every hull the
    debtor helms or owns (Q-50-68).
  - A license is kept by gateway account, so the bar on a new one binds the account (Q-50-68).
- **Alternative:** fines only in the first cut.
- **Why:** the owner named all three. Each has a working precedent: port security's fine and
  quarantine, standing grants with a cause, and the Roll's 90-day bar.

**Q-50-40 — Seeing it and resolving it.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** `+record`, one page for the actor. Each open matter shows what it is, which
  polity proved it and by what kind of evidence (never who reported), the fine and its grace in IC
  and real time, the restriction in force, and exactly what lifts it. `+debts/pay` settles, as
  today. A queued notice and IC mail go out when a matter opens.
- **Correction (outside review, doc 50a C-7, A-10e):** the notice is an official notice, delivered
  at once. What the page and `+debts` cover is Q-50-70.
- **Alternative:** `+debts` and `+standing` as they are.
- **Why:** the owner's words are "a restriction they can see and resolve".

**Q-50-41 — Contesting a matter.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** no appeal in the first cut. A matter is proven or it is not written.
- **Alternative:** a contest window of 28 IC days before the fine posts, heard by the claim-contest
  machinery or by staff.
- **Why:** the proof standard is high by construction: an authority's own check or a witness's own
  record. An appeal needs an adjudicator, and the storyteller role is disconnected (Q-44-29).

**Q-50-42 — How standing is earned back.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** no decay, as built. Paying a fine restores nothing. Lawful work for that
  polity earns standing: survey commissions do today, and delivering to its stockpile or escorting
  its hulls would.
- **Correction (outside review, doc 50a B-1):** not a grant for each delivery. The standing store
  keeps every receipt and holds 16,384 of them. Work earns one grant a polity an IC month
  (Q-50-69).
- **Alternative:** decay toward neutral over a provisional 90 IC days.
- **Why:** grants are immutable and stamped with a cause by design. Decay is a hidden clock.
  Work for standing ties this part to §6.3.

**Q-50-43 — A witness's reward.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** a witness whose report proves an offense is paid a share of the fine when
  it is collected (provisional 20%), and is never named to the offender.
- **Alternative:** no reward.
- **Why:** it gives a player an inspector's interest on the route. Q-38-20 left bounties for later;
  this is the smallest one. A false report is not possible when the report must carry the
  witness's own record. Betrayal stays legal, recorded and discoverable, as the faction design
  asks.
- **Correction (outside review, doc 50a B-5):** "a false report is not possible" was wrong as
  written. The market's record holds no IC time and nothing about the seller's papers, so a seller
  whose license lapsed afterward could be reported, and one who bought a license afterward could
  not. It becomes true when the receipt of Q-50-66 records the papers at the sale. A reward that
  costs the buyer nothing also invites buying at a discount and informing for the rebate; Q-50-67
  makes the reporting buyer hand over the goods.

**Q-50-44 — Seized goods.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** credited to the enforcing authority's vault and offered at its port, so
  matter is conserved.
- **Alternative:** destroyed, and counted in the matter census under a `seized` term.
- **Why:** today the picket's seizure debits the hold to nobody. Seized ore for sale at a discount
  is something a trader can find.
- **Correction (outside review, doc 50a C-8, A-5):** the census already has the `seized` term the
  alternative names; nothing writes it. Under the default a seizure is a transfer, and that term
  stays at zero. A seizure also has to be able to take cargo under a haul's hold (Q-50-25).

### 6.5 Order, size and proof

**Q-50-45 — Order.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** first the model (Q-50-1 to Q-50-5) with the Kirkwood cut as its first
  reader. Beside it, law's first cut: one good, one port, the record and the page; the two share
  only the schedule. The Isidis operation last, because it needs the place, organization posting,
  loss handling, EX.10 and the bubble rule.
- **Alternative:** Isidis before law.
- **Why:** the owner called Kirkwood the small first cut. Law reuses port security. Isidis carries
  most of the new work.

**Q-50-46 — Sizes at 24:1.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** a front moves over real days, not hours. Provisional: Customs Assertion at
  six segments; the Review's tick one an IC month; the operation's notice and window 28 IC days
  each. All tuned after the playtest.
- **Alternative:** the longest notice and window the engine allows, 84 IC days each (three and a
  half real days each).
- **Why:** a rate per IC day is felt per real hour (Q-48-15's lesson). The playtest will say
  whether 56 real hours is a weekend's campaign or too quick to notice.

**Q-50-47 — The acceptance stories.** **Ruled 2026-11-19: accepted as corrected.**
- **Proposed default:** three, each on production command paths at 24:1 with two accounts.
  - A hauler files the same run before and after the Kirkwood flip, and reads the duty and its
    reason.
  - A player who never joined arrives at Isidis after the operation, and reads who won from the
    terms.
  - A smuggler's private sale is reported by its buyer, fined, restricted, paid and cleared.
  
  And one run with nobody connected: both fronts move and the operation resolves.
- **Correction (outside review, doc 50a B-1, A-2e, B-3c):**
  - The third story, made exact: the smuggler cannot pay at once. The unpaid fine restricts them,
    paying it lifts the restriction, and the standing loss stays and shows on `+record`.
  - The empty run cannot pass on the engines as built: a resolution between organizations
    advances no clock, and no NPC hull answers the picket. It waits on Q-50-51, Q-50-56 and
    Q-50-57.
- **Alternative:** unit tests a piece.
- **Why:** the owner stated the first two as milestones. The empty run is the "no faction-head
  bottleneck" test.

**Q-50-48 — The bot.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** the mining bot learns the duty line and the `+record` page, and its
  sentence table gains each new sentence. No bot lane plays the operation in the first build.
- **Alternative:** a bot cohort hauls for the stockpile in the nightly.
- **Why:** docs 47 to 49 hold every unit to the sentence table. A campaign lane is clock-bound and
  slow, and doc 43's clock seam (TS.10) is not built.

**Q-50-49 — The other fronts.** **Ruled 2026-11-19: accepted as written.**
- **Proposed default:** out of scope. Registry Divergence, The Slowdown and the relay fronts keep
  their text outcomes.
- **Alternative:** give every shipped front a control outcome now.
- **Why:** The Slowdown's outcome ("throughput falls, prices spike") is a second schedule row once
  the model exists, and is better sized after the first three have been played.

### 6.6 Raised by the outside review (doc 50a)

Twenty-two follow-ups, all ruled, accepted as proposed: seventeen by the owner's second ruling,
and Q-50-55, -57, -62, -65 and -67, which the lead had held for his own call, by the third.
Each names the triage row that raised it.

#### The model and the faction engine

**Q-50-50 — One de facto holder a region, and the date its hold began.** (doc 50a A-1)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** de facto control is exclusive. A flip, by a clock or by staff, moves the
  prior de facto holder out. De jure, covert and economic control stay plural, as doc 11 §5 uses
  them. The control record gains the IC date of the hold, set at the flip, and `+territory` shows
  it. The faction crate gains one public function that seeds and flips control, for the lane's
  seed and for TacSim's own typed calls. This is work in `mush-faction`.
- **Alternative:** leave the engine as it is. TacSim treats the newest de facto record as the
  holder.
- **Why:** Q-50-4 reads "the de facto holder", and today a flip leaves two with no marker of which
  is current. The reason clause of Q-50-9 needs an IC date, and the record holds wall-clock time
  that nothing reads.

**Q-50-51 — A front's lifecycle, and what moves a polity's clock.** (doc 50a A-1, A-2d, A-2e)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** three changes to the engine.
  - **One completion path.** Every clock in this plan completes through the engine's own path, and
    that path marks the front resolved when its last clock fills. Today the engine's path flips
    control and leaves the front open, and the hand-written paths close the front and flip
    nothing.
  - **A front can be reopened and a clock set**, by a call for compiled owners and a verb for
    staff. Q-50-31's repeat and any later push-back need it.
  - **A polity's clock moves by a typed owner call that carries a cause id**, as the customs path
    does today. It does not move by the canon observer.
- **Alternative:** a repeat deletes and re-creates its clocks; and clocks are bound to canon kinds,
  each kind added to the engine's list of kinds that need no member.
- **Why:** the observer advances a clock only when the record names a member of the clock's
  faction, and reads the record's kind alone. A resolution between two organizations names no
  member, so the empty run would move nothing. The standing rule is already that a clock advances
  "only from a typed cause with a stored cause id".

**Q-50-52 — Papers where two polities claim de jure.** (doc 50a D-6)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** doc 11 §9 question 2 is taken at its recommended answer: Isidis is seeded
  with two de jure claimants, the Republic and the Freeport. An enforcer reads its own polity's
  papers where that polity is a de jure claimant, and otherwise the de jure holder's. So at
  Kirkwood after the flip the Republic's picket still reads the Freeport's license; at Isidis each
  administration would read its own. Isidis needs no license in the first build: its schedule
  carries an import duty and no restricted good.
- **Alternative:** one de jure holder a region (doc 11 §9 question 2, option B), and the question
  never arises.
- **Why:** Q-50-4 says licenses follow de jure, which is one answer only where de jure is one
  holder. Doc 11 §5's table already assumes two at Isidis.
- **With this ruled (2026-11-19):** doc 11 §9 question 2 is answered at its option A. Doc 11
  carries a note there.

#### The Kirkwood cut

**Q-50-53 — What the transit duty is assessed on.** (doc 50a A-4; the review's item 2)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:**
  - **Cargo is dutiable:** every good in the hold, pooled or held as a lot, and cargo under a
    haul's hold.
  - **Ship's stores are not:** propellant and provisions in the hull's own compartments that are
    neither a lot nor under a haul's hold. Bay vehicles and a towed hull are not cargo.
  - **The value is the good's authored baseline** in the catalog (600 cr a tonne for strategic
    ore), not a live ask. A good whose baseline is a placeholder of 1 cr pays only the floor.
  - **The duty is on cargo, not on a flag.** A Republic hull pays it too.
- **Alternative:** value at the live reference the Co-op uses, which is the named house's ask (800
  cr for strategic ore today).
- **Why:** Q-50-8 and Q-50-10 need one number that is the same in the estimate, at the filing and
  at the picket. A house's ask steps every IC week. One known gap: unreserved propellant carried
  for sale passes as stores. It is accepted for the first cut and flagged for tuning. §8 notes
  that the survey has not checked which hulls can carry which goods as cargo.

**Q-50-54 — A pilot who cannot pay the duty, and a pilot who is away.** (the review's item 1)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** the hull is cleared, and the duty becomes a debt to the customs office with
  28 IC days of grace, then the penalties of Q-50-39. Lawful cargo is never seized for a duty. A
  hull whose pilot is away at the picket, on a queued departure or with a lost link, is assessed
  the same way with no submission, and cleared. Running the picket stays a choice only a present
  pilot can make.
- **Alternative:** cargo to the value of the duty is seized.
- **Why:** at 24:1 a passage spans real hours, and a pilot who logged off must not be left in a
  bubble. Today's answer for a pilot who cannot pay is a seizure, which fits contraband and not a
  tax. What an absent pilot's hull does at the picket today was not traced to the end (§8).

**Q-50-55 — The ways around the picket.** (doc 50a B-3a, B-3b)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:**
  - **The long way round stays open and carries no picket.** Ceres to Luna to Deimos to Stickney
    is the lawful detour: the duty buys the direct run, and the detour costs time and propellant.
    KC.3's story measures both, and the duty is tuned so the direct run is still worth taking for
    ordinary cargo.
  - **The shortcut closes.** A free-flight autopilot from Ceres' halo to Deimos owes no leg today
    (§4.3). It is brought under the lane's routes: it flies the long way's legs or it owes the
    picket's leg. The unit decides which once the autopilot's planning has been read (§8).
  - **The outward run stays free**, as built.
- **Alternative:** a second picket on the Deimos approach, so that every inward route is assessed.
- **Why:** a duty with a detour that costs time is a real choice between money and hours. A duty
  with a free shortcut is no duty. The shortcut is also a way past the contraband check today.

**Q-50-56 — NPC hulls at the picket.** (doc 50a B-3c) **Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** an NPC hull answers the picket by rule, in compiled code. It heaves to, is
  assessed, and its house pays the duty or the fine from its own account. No NPC hauler is sent
  the long way only because it cannot answer. EX.10's hauling house is built to this rule.
- **Alternative:** NPC haulers keep to the long way round, as Tycho Mutual's retrieval arm does
  today, and never meet the picket.
- **Why:** the empty run of Q-50-47 needs an NPC hull to cross the approach. An NPC hauler that
  never meets the picket can never be the seized hauler of Q-50-20, and pays no duty a player
  pays.

#### The Isidis operation

**Q-50-57 — What the campaign engine must gain to frame the operation.** (doc 50a A-2)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** `mush-campaign` stays the frame, with five additions built as their own
  unit before any arm uses it. **(expensive to reverse)**
  1. **A public way to declare and activate**, so a compiled arm does both in one step. That also
     steps round the "unfunded" defect of §4.3.
  2. **Organizations as declarer, sides and defenders**, with the defender's notice sent to the
     organization's members.
  3. **An objective read only at the window's close**, with its own polarity. For Isidis: "the
     tank farm is at its threshold at close" is the defenders' success. No objective in this plan
     reads canon.
  4. **The outcome reaches TacSim through a seam at the composition root:** a TacSim trait,
     implemented in `mush-composition` over the campaign crate's public API, as the crate's other
     links are built. TacSim then moves the control record by its own typed call (Q-50-51).
     Neither crate's pinned dependencies change.
  5. **The bond is forfeit to the defenders' treasury when the declarer fails** (provisional).
     Today it is released whatever happens, so declaring costs nothing.
- **Alternative:** no campaign engine. TacSim keeps a small operation record of its own (declared,
  notice, window, sides, outcome) beside the two fronts.
- **Why:** the review showed five ways the engine as built cannot do this. Q-45-15 still rules
  that a change of control is an operation with notice, and the engine already holds the notice,
  the window, the bond and the three views. The alternative is less engine work and a second model
  of an operation. The owner should choose which is the frame before OP is cut.
- **With this ruled (2026-11-19):** the campaign engine is the frame. OP.0 is the five additions
  and the seam, and the TacSim-only record is not built.

**Q-50-58 — Isidis's routes, and the tank farm's price.** (the review's item 12)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** Isidis is reached from Stickney alone, by one short route each way with no
  picket. Belt cargo for Isidis therefore arrives by the direct run or by the long way. The
  stockpile is **propellant only** in the first operation. The tank farm's standing bid is priced
  above Ceres' ask and below the Mars-orbit asks (provisional: between 40 and 60 cr), so
  propellant bought at Stickney or Deimos and landed at Isidis loses money, and propellant hauled
  from Ceres earns. No rule forbids either.
- **Alternative:** the Republic's houses in Mars orbit refuse the stockpile goods to anyone while
  the window is open.
- **Why:** Stickney and Deimos sell propellant at 60 cr and Ceres at 40, so price can steer it.
  Provisions cost 10 cr at all three, so price cannot, and they leave the list. Without this a
  player fills the Freeport's tank farm from the Republic's own shelves and never crosses the
  approach.

**Q-50-59 — Where the import duty falls.** (the review's item 12)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** at sale. When a good on the schedule's list is sold at Isidis's venue,
  into a bid or from a listing, the duty comes out of the seller's proceeds through the proceeds
  policy, and the quote shows it with its reason. Cargo that is landed and stored pays nothing
  until it is sold there. The list is authored in the schedule; provisional: the goods the Belt's
  ports export, which are the ores, the volatiles and propellant.
- **Alternative:** at docking, assessed on the hold as the transit duty is.
- **Why:** the milestone is a newcomer who reads who won "from the terms they're offered", and the
  offer a newcomer reads is a bid's quote. The seam already sees the seller, the venue, the good
  and the price.

**Q-50-60 — What a repeat starts from.** (the review's item 12)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** the tank farm drains. The garrison and the landing field draw it down at a
  steady provisional rate, on the pattern of EX.9's consumption arm, so the level at each window's
  close reflects what was landed during that campaign.
- **Alternative:** the objective counts only what was landed inside the window.
- **Why:** without a drain the Freeport begins the second operation already past its threshold. A
  drain also makes holding the Concession a standing cost, which gives the hauling work a reason
  to continue between operations.

**Q-50-61 — Whose side a hull is on.** (doc 50a B-2; the review's item 13)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** a hull's flag is the polity of whoever holds its title or its charter. An
  organization's hull takes that organization's polity from the lane, a Co-op Dray is
  Freeport-flagged, and a personally titled hull flies none. During an operation a hull flies for
  a side when it carries a live job of that side's arm, a haul or an escort, or when its flag is
  that side's polity. Nobody joins by a command.
- **Alternative:** the pilot's faction memberships decide.
- **Why:** title, charter and job are records that exist and that a player can read. Under this
  rule the Review's "seized Freeport-flagged hauler" is a seizure ruling on a Co-op Dray, which
  new captains do fly past the picket.

**Q-50-62 — Leaving a fight in an operation's bubble.** (doc 50a A-9)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** a hull flying for a side may break off. Holding outside a provisional
  distance from every hostile hull for a provisional time releases it to resume its passage. It
  loses the engagement and nothing else. Q-50-26 is not built without this.
- **Alternative:** Q-50-26's alternative A for the first operation: no fire between players'
  hulls.
- **Why:** a bubble holds a hull while any operational hostile hull remains, and today the only
  way out is to disable one. That is tolerable against a picket that stands down. It is not
  between two players, one of whom may have logged off.
- **With this ruled (2026-11-19):** Q-50-26 is built for the first operation, with this way out.

**Q-50-63 — Escorts for an organization's hauls.** (doc 50a A-8)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** an escort may be posted for a haul job as well as for a hull. Posted with
  the haul, by the arm or `as=<org>`, it binds to the hauler's hull when the haul is taken. It
  runs to the haul's destination across every hop. The escort has until the protected hull has
  launched, plus a provisional grace, to be under way. The same change repairs the two escort
  defects of §4.3.
- **Alternative:** the hauler posts their own escort and pays it from the haul's fee.
- **Why:** today a post names a hull the poster holds. An organization cannot do that, and nobody
  can before the hauler is known.

**Q-50-64 — When a port changes hands: who is paid.** (doc 50a A-10d; the review's item 14)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** in the first operation only the import duty follows the holder. Isidis's
  venue fees, berth, locker, crew wages and release fee stay with the port's authored authority.
- **Alternative:** every payee follows the de facto holder: new charges at once, standing berth
  and locker obligations at their next term, and the clerk's release capability granted on the new
  authority's scope.
- **Why:** the port's authority has eight consumers. Two never re-point, and one is a grant made
  once a world. The milestone is met by the duty alone. This reverses the last sentence of
  Q-50-29's default.

#### Law

**Q-50-65 — Possession, the papers, and the buyer on the board.** (doc 50a C-1; the review's item 6)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** one test, the picket's, as built: a prospecting license and a registered
  claim on a strategic rock in an orbital cluster, both. It is asked in three places.
  - **The public board** refuses a holder without the papers on either side of a trade in a
    restricted good, selling or buying.
  - **The picket** reads it for cargo, as today.
  - **A port audit** reads it for the hold. Holding unlicensed strategic ore in a docked hold at
    Ceres is itself an offense, found only by an audit.
- **Alternative:** a license alone covers buying, carrying and selling ore that was bought. The
  claim is asked only of ore the hull mined.
- **Why:** refusing an unlicensed buyer on the board means nobody acquires contraband from a house
  by accident. It then arises only from unlicensed mining and from private sales. The alternative
  opens a merchant's trade in strategic ore, and the owner may want that. A surface claim counts
  for nothing either way, as built.

**Q-50-66 — The receipt of a private sale.** (doc 50a B-5)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** when a good the venue's law restricts is sold privately, settlement writes
  a TacSim receipt for each party: seller, buyer, venue, good, tonnage, price, the IC time, and
  whether the seller held the papers at that moment. Each party reads their own on `+record`. A
  witness report cites the receipt, which must be no more than one IC day old. Nobody else can
  witness a private sale. The receipt alone fines nobody.
- **Alternative:** no witness reports of sales in the first cut. Only an inspection proves an
  offense.
- **Why:** the market's own record carries wall-clock time and no papers, and it is evicted. With
  the papers recorded at the sale, a report cannot be made false by a license that lapses or is
  bought afterward. It stays within Q-S4: a record a party holds, acted on only when that party
  brings it.

**Q-50-67 — The buyer who reports.** (doc 50a B-5) **Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** a buyer who reports hands over the lot, which goes to the authority's vault
  (Q-50-44). When the fine is collected the buyer is repaid the price they paid, and paid the
  reward of Q-50-43, both out of the fine and never more than it. A buyer who keeps the goods
  holds contraband (Q-50-65).
- **Alternative:** the buyer keeps the goods and takes the reward.
- **Why:** under the alternative a buyer takes the discount and the reward at no cost, and no
  seller would sell privately twice. With the goods surrendered, informing is a trade a player
  chooses: an inspector's sting, repaid only if the fine is collected.

**Q-50-68 — Who answers: character, organization, account and hull.** (doc 50a A-10b, A-10c)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:**
  - **The record is written against whoever acted:** the character for their own act, the
    organization for an act made `as=<org>`. The fine is drawn on that principal's account.
  - **A hull's hold answers through its pilot at docking,** and an organization's hull through
    the organization. Today an organization's hull answers to nobody.
  - **A quarantine holds the hull that docked,** at the ports the proving polity holds. Today a
    past-due fine holds every hull its debtor helms or owns, at every port and site.
  - **The bar on a new license binds the gateway account,** because a license is kept by account.
  - **An offender with no hull is fined when the offense is proven,** with no docking needed.
- **Alternative:** every record is the character's, in whatever capacity they acted.
- **Why:** Q-50-37's key is "actor by polity" and marked expensive to reverse, so the actor has to
  be defined before it is built.

**Q-50-69 — Standing: the threshold, the sizes and the way back.** (doc 50a B-1)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:**
  - **"Below neutral" is the unfriendly band or lower:** a score of −10 or less.
  - **Sizes, provisional:** nothing for an offense an authority's own inspection found and the
    offender paid at once; −5 for a sale proven by a witness; −10 for running the picket. Two
    proven sales, or one run, restrict.
  - **The audit chance by band, provisional:** never when trusted, one in eight when friendly, one
    in four when neutral, one in two when unfriendly, always when hostile.
  - **Lawful work earns one grant a polity an IC month,** sized by the work done in it.
- **Alternative:** Draft 0.1's sizes, −3 and −5, with the restriction beginning at any score
  below zero.
- **Why:** neutral runs from −9 to 9, so Draft 0.1's sizes left an offender neutral after three
  proven sales. The standing store holds 16,384 receipts and never evicts, so a grant for each
  delivery would fill it.

**Q-50-70 — What `+record` and `+debts` cover.** (doc 50a A-10e; the review's item 10)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** `+record` lists proven offenses, open fines with their grace, restrictions
  in force, the viewer's own receipts (Q-50-66), and a core under a live Registry impound on a
  hull the viewer holds. A forfeited core is not listed, because the seizure's row names no owner.
  Registry mind fines join `+debts` and `+debts/pay`, which do not show them today.
- **Alternative:** an owner index on the Registry's seizures, so past forfeitures are listed too.
- **Why:** the Roll's families are closed (doc 46). The page reads them and does not move them.

**Q-50-71 — Fees on a private sale of a restricted good.** (doc 50a B-4)
**Ruled 2026-11-19: accepted as proposed.**
- **Proposed default:** as built. A private ask pays the venue's listing fee and cut whatever is
  sold, and a private bid sold into pays nothing. The fee's ledger row is evidence of nothing.
- **Alternative:** a private trade in a restricted good pays no venue fee; or a private bid pays
  the cut as a private ask does.
- **Why:** the port already takes its cut of every private ask, so a black market would pay the
  port. That is odd and harmless. Changing it is a choice for the whole market, and belongs with
  doc 48's fee rulings.

## 7. Units of work (a sketch; cut only after the rulings)

Three chains. The names and the split are provisional. Draft 0.2 adds the work the outside review
showed the first units need; it is marked **(0.2)**.

**Where the units stand at Draft 0.4.** Every question is ruled, so no unit waits on one. No
unit is cut; that waits on the owner's word. The order within a chain is unchanged. What still
gates a unit is other work: the cargo reading and the defects named under "Edges with work in
flight" below, EX.10 for OP.4, and OP.0 for the units that stand on the frame.

**Before the chains (0.2): the cargo reading.** The picket, the hold's fill check and every new
reader of a hold count a bulk good held as a lot (§4.3). KC.2, LW.3 and OP.3 all wait on it.

**KC: control and the Kirkwood cut**
- KC.0a **(0.2)**: the faction engine: one de facto holder, the IC date, one completion path, a
  front reopened and a clock set, a public call that writes control (Q-50-50, Q-50-51).
- KC.0: the six polities as factions from content and the table from lane organization to polity;
  the lane's `regions` block; the control read (Q-50-1 to Q-50-4, Q-50-52).
- KC.1: the customs front on the Republic's board, seeded at world start, completing through the
  engine's path with a control flip (Q-50-12, Q-50-13).
- KC.2: the terms schedule; the transit duty in the customs ruling and what it is assessed on; a
  duty unpaid or a pilot away; pinning at filing and at an autopilot engagement; the shortcut
  closed; the payee (Q-50-5, Q-50-6, Q-50-10, Q-50-11, Q-50-14, Q-50-53 to Q-50-55).
- KC.3: the duty line on `+ship/plan`, the filing and the job board, and the reason clause; the
  first acceptance story, which also measures the long way round (Q-50-8, Q-50-9).
- KC.4: the Co-op's enrollment share by control (Q-50-7).

**LW: law**
- LW.1: the law asked for a venue and a good through the proceeds policy; the board's refusal on
  both public paths and of the buyer (Q-50-32 to Q-50-34, Q-50-65, Q-50-71).
- LW.2: the record of proven offenses and who answers; jurisdiction by control; port security
  moved onto it, with its quarantine narrowed (Q-50-37, Q-50-38, Q-50-68).
- LW.3: the port audit on the port tick; the receipt of a private sale; the witness report, its
  reward and the reporting buyer (Q-50-35, Q-50-36, Q-50-43, Q-50-66, Q-50-67).
- LW.4: the standing loss and its sizes, the trading restriction, `+record` and `+debts` (Q-50-39
  to Q-50-42, Q-50-69, Q-50-70).
- LW.5: seized goods to a vault, cargo under a haul's hold included (Q-50-44).

**OP: the Isidis operation**
- OP.0 **(0.2)**: the campaign engine's five additions and the seam at the composition root
  (Q-50-57, ruled at its default).
- OP.1: Isidis as a place, with its routes (Q-50-17, Q-50-58).
- OP.2: the scheduled tick as a typed cause; the Concession Review and Hold the Concession; the
  hull's flag (Q-50-20, Q-50-61).
- OP.3: posting as an organization; escorts for a haul; the haul's loss handling (Q-50-23,
  Q-50-25, Q-50-63).
- OP.4: the stockpile, its drain and the Freeport arm's budget; NPC hulls at the picket (Q-50-21,
  Q-50-22, Q-50-24, Q-50-56, Q-50-60).
- OP.5: the declaring arm, the operation frame and the resolution into control and the import duty
  (Q-50-18, Q-50-19, Q-50-28, Q-50-29, Q-50-59, Q-50-64).
- OP.6: legal targets in a bubble, and breaking off (Q-50-26, Q-50-62).
- OP.7: `+operation`, the notices, the Republic's bid (Q-50-27, Q-50-30).
- OP.8: the empty run and the second acceptance story (Q-50-47).

**Edges with work in flight.** Doc 48's EX.9 owns the consumption arm and the lane's stock rows,
and EX.10 the NPC hauling house, which OP.4 leans on and which must answer the picket (Q-50-56).
Every KC and LW unit that adds a schedule row bumps the lane, so those run one at a time, as doc
48's lane chain did. The eleven live defects of §4.3 belong to the fix thread, and three of them
gate units here: the cargo reading, the seizure under a haul's hold, and the two escort defects.

## 8. What this survey did not check

**Answered by the outside review's lanes (doc 50a §4), and no longer open:**
- **Where a passage is priced before launch:** `+ship/plan`, a dry run with no money on it.
- **Whether a lot records its license or its origin:** no, by the inventory's own rule.
- **The trade tape's visibility:** private trades are off `+market/depth`, and there is no tape
  verb.
- **A seeded roll in the port tick:** none; the precedents are a keyed hash.
- **The escort fact in code:** read; §3.4 and §4.3 hold what was found.
- **Doc 11 §9's questions:** unruled when read. Q-50-52, ruled at Draft 0.3, answers question 2
  at its recommended option. The others stay unruled.
- **How `+territory` reads today:** "No territory is mapped."

**Still not checked:**
- **Which hulls can carry which goods as cargo.** A lane found that the Dray's cargo compartment
  takes one stowage class and that strategic ore is another. Which hulls can carry strategic ore,
  and whether propellant can be carried beyond a hull's own tanks, decides who can run the ore
  and who can haul for the tank farm (Q-50-32, Q-50-53, Q-50-58).
- **Whether ore that was only mined can be made into a lot.** A lane inferred that it cannot,
  from the code, and did not test it.
- **What a hull left "not cleared" may still do**, and whether it can move.
- **How a free-flight autopilot plans a passage** between two ports the lane gives no route
  (Q-50-55).
- **What an absent pilot's hull does at the picket today.** The link-loss record's writers were
  read; the drift and what follows were inferred (Q-50-54).
- **Whether the quarantine at every port is meant** (§4.3).
- **How long a Registry mind fine's one period of grace is**, and how long economy contracts and
  ledger rows are kept as a record of a sale.
- **Whether a past wall-clock time can be turned back into an IC date.** The review says it
  cannot across a re-anchor; no lane read that code. Q-50-50 stores the IC date and does not
  depend on it.
- **What EX.9 and EX.10 will have changed** by the time these units are cut: the faucet, the
  stipend and the hauling house are in flight.
- **What the dev boxes run.** Nothing here was tried on a live server.
