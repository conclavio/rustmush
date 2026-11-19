# 50. Law and the corridor — control that sets terms, a faction operation players feed, and offenses with consequences, surveyed

Draft 0.1, 2026-11-18, docs only. Surveyed against `origin/main` at `1e9b41174` by five read-only
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

Anchors are file names with line numbers; line numbers drift.

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
7. **A faction cannot post a job.** A haul or an escort is posted by a character standing in a
   port, from their own locker to their own locker, paid from their own account. No organization
   and no NPC can post one. The haul code handles no wreck, capture or seizure.
8. **Isidis is prose.** It is named in two NPCs' facts and two rooms' text. There is no port, site,
   region, organization or front for it. Two of the three fronts doc 11 gives the corridor (the
   Concession Review and Hold the Concession) are not in the code.
9. **Legality is a field nothing sets.** The market records legal, restricted or forbidden per
   listing, and every TacSim catalog row hard-codes legal. The venue gate does not see the good.
10. **Enforcement exists in three separate places.** The picket fines or seizes unlicensed
    strategic ore. Port security fines a proven salvage offense at docking and quarantines the
    hull until paid. The Registry sanctions an ungoverned core. Each has its own record, its own
    proof and its own payee, and two of them name their ports in a constant.
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
- **Fronts are made** by staff verbs or by Rust seed functions. No content file defines one.
- **TacSim seeds no region and no control record**, and no file outside `mush-faction` and
  `mush-campaign` names `ControlRecord`. TacSim uses the faction crate for standing, stances and
  the fronts below.
- **The polities are not content.** Each lane organization carries a `faction` tag (`freeport`,
  `republic`, `concord`, `syndicate`, `company`) that is "carried and validated, read by nothing
  until the faction cut (lead ruling L16)" (`port/def.rs:1081-1083`). The Republic faction and the
  three Houses are made by staff in the launch bring-up.

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
  Strategic ore is unlicensed when the pilot or principal holds no Freeport Registry prospecting
  license and no registered claim. Nothing else is contraband there.
- **The ruling** is one of Cleared, Fined, Seized or Ran, written by the action
  `tacsim_space_customs` (`space/customs.rs`).
- **The fine** is 500 cr a tonne begun, never under 1,000 cr, chosen when the pilot can pay. It is
  paid to the **Freeport Registry**, although the line the pilot reads names a Republic Customs
  corvette (`picket.rs:1402-1492`, `customs.rs:834`).
- **The seizure**, when the pilot cannot pay, debits the ore from the hold in the ruling's own
  transaction. It is credited to nobody (`customs.rs:466-484`).
- **A run** (`+helm/run`, or burning past 35 km) turns the corvette hostile.
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

- **There is no tax, tariff, toll or duty on ordinary cargo**, and no passage, clearance or docking
  fee. "Clearance" in the voyage code is a launch gate.
- **"Republic fees" and "Co-op rates" are words in doc 11 and in room text.** No code or content
  defines a fee that differs by who holds a place. The Co-op's real terms are a stake of 25,000 cr,
  the share above and a buyout of 90,000 cr, all in the lane's `programme` block.
- **A port's `authority`** is one of its organizations, named in the lane pack: the Freeport
  Registry at Ceres, Stickney Yards at Stickney, and one each at Luna, Deimos and Titan. It is
  baked into the pack's version and digest. Nothing changes it while the server runs.
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
- **No TacSim source calls it.** The one seam a domain has is to commit canon, which the campaign
  tick reads.
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
- **Doc 48's EX.10**, not built, promises an NPC hauling house whose real hulls run between ports
  when a price gap pays, and can be intercepted.
- **There is no stockpile, quota or requisition.** Goods sit in vault pools. The nearest ideas are
  the hull pool's floor, the intake drip's cap and EX.8's stock target.
- **Losses** leave their own records: a customs ruling, a hull's end state in space (operational,
  immobile, disabled, wreck), a wreck on a site, a capture. None reaches an operation unless it
  also commits canon.
- **In a space bubble every carrier is friendly to every other** (`space/tick.rs:5273`, `:5894`).
  One player's ship cannot fire on another's. Doc 47 left this as an open design call.
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
| Port security (RS.7) | a proven salvage row | a vacated contest, or a witness report with the witness's own record under one IC day old | a fine at docking at Ceres or Stickney; unpaid, a debt and the hull quarantined |
| The bay | a part stripped unlawfully in the field | the strip's own marker | `+bay/fit` refuses the part until its owner clears it |
| The field | friendly fire; capture | the field's black box | standing lost, voidable by the leader |
| The Registry (minds) | an ungoverned core, a grey die | `@registry/inspect` | a sanction, a 90 IC day bar, the core impounded in place |

- **Port security's ports are a constant**, `SECURITY_VENUES = ["ceres", "stickney"]`, and its
  fines (500, 2,000 and 5,000 cr) are paid to the Freeport Registry at both
  (`surface/security.rs:40-46`).
- **Legality.** `mush_market::Legality` is `Legal`, `Restricted` ("sold under a license") or
  `Forbidden`. Its comment: "Recorded, rendered and filtered on; never enforced by this crate —
  jurisdiction is a policy test at transaction time (10b §3.2), and the ruleset owns it." Every
  TacSim catalog row writes `legal`. The venue gate admits by actor, venue, endpoint and presence;
  it does not see the good.
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
  "proven rows only". Q-41-25: the fine at docking, the shortfall an obligation, the hull
  quarantined until it settles.
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
  it looked like later. Mail is in character and must transit.
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
- **A seizure destroys matter.** The ore is debited and credited to nobody, and doc 48's matter
  census has no term for it (48-cut-notes, EX.0).
- **A Republic corvette collects for the Freeport Registry.** It may be intended (the license is
  the Freeport's). Q-50-14 asks.
- **A salvage fine at Stickney is paid to the Freeport Registry**, by constant.
- **Doc 11 against the code:** the customs clock is four segments in the text and six in code; the
  front is the Republic's in the text and on the Registry's board in code.
- **Doc 11 line 254** still says the relay backbone "is not built". Doc 15's R1 to R10 are built.
- **GOD-owned NPCs act for each other** (48-cut-notes, EX.3), so the market refuses a trade between
  two of them as a self-trade. Any NPC arm in this plan needs it fixed, or an agent object of its
  own as procurement uses.

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

## 6. Owner survey

Every question has a proposed default, an alternative and a reason. Nothing here is ruled.

### 6.1 The model

**Q-50-1 — Where control lives.**
- **Proposed default:** the faction engine's regions and control records are the one record of who
  holds a place. TacSim reads them through one function, and a front's completion flips one through
  the engine's own `on_complete_control`. **(expensive to reverse)**
- **Alternative:** a TacSim record per port or lane node; or a terms overlay written by each front.
- **Why:** the engine was built for this and its rule is already ruled (control changes only by a
  clock or by staff). `+territory` shows it. Doc 11 §5 is written in its four kinds. A second
  record would be a twin of the first.

**Q-50-2 — The polities as factions, from content.**
- **Proposed default:** at boot each lane organization's `faction` tag is bound to a faction of
  that name, made from content when it is missing. The bring-up stops making the Republic by hand.
  The Houses stay organizations under their polity.
- **Alternative:** keep the staff-made factions, and name them to TacSim in the instance config.
- **Why:** the tag has waited for "the faction cut" since lead ruling L16. A front on the
  Republic's board needs the Republic to exist on a fresh world with no staff step.

**Q-50-3 — Regions, from content.**
- **Proposed default:** the lane pack gains a `regions` block: one row for each doc 11 §5 row the
  game uses, naming its places (port entries, lane nodes, sites) and its four holders at world
  start. The first cut seeds the Kirkwood approach, Ceres and Stickney. Isidis arrives with its own
  unit. One lane bump.
- **Alternative:** staff make regions with `@region` at bring-up.
- **Why:** control has to exist on a fresh world. Content is reviewed and pinned by digest.

**Q-50-4 — Which kind of control sets what.**
- **Proposed default:** fees and enforcement follow **de facto** control. Licenses, permits and
  claims follow **de jure**. Covert and economic control are recorded and read by nothing yet.
  **(expensive to reverse)**
- **Alternative:** one holder a region, with no kinds.
- **Why:** the Kirkwood row needs the split: the Freeport licenses, the Co-op runs the depot, the
  Republic contests. Customs Assertion's outcome is written as "de facto". The split is also what
  gives a smuggler an argument: licensed by one authority and taxed by another.

**Q-50-5 — How terms hang off control.**
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

**Q-50-6 — What "Republic fees apply to the lane" is.**
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

**Q-50-7 — What "the Co-op's rates rise" is.**
- **Proposed default:** new Co-op enrollments sign at a higher share while the Republic holds the
  approach (provisional 300‰ against 250‰). Standing enrollments keep the terms frozen at signing.
- **Alternative A:** a surcharge on every chartered sale while the hold stands.
- **Alternative B:** leave it as prose in the first cut; the duty is the price change.
- **Why:** the share and the buyout are frozen on the enrollment, which is the charter's promise.
  The duty already reaches every hauler, chartered or not.

**Q-50-8 — Which quote shows it.**
- **Proposed default:** three places. Where a passage through Kirkwood is planned or filed, a line
  with the duty estimated for the cargo aboard. The picket's ruling line. The job board, where a
  haul routed through Kirkwood shows the duty its hauler will meet.
- **Alternative:** the ruling line only.
- **Why:** the owner's test is the same run before and after, and a hauler decides before launch.
  §8 notes the survey did not find where a passage is priced before launch.

**Q-50-9 — The reason line.**
- **Proposed default:** every charge that control sets carries a clause naming the holder and the
  IC date its hold began, within 120 columns. `+territory` and `+chronicle` show the same record.
- **Alternative:** a `+terms <place>` page, and no clause on the charge.
- **Why:** no surface in the game says why a price is what it is. The owner asked that a hauler
  "be able to find out why".

**Q-50-10 — A passage under way when control flips.**
- **Proposed default:** terms are pinned when a passage is filed. A hull that launched before the
  flip clears under the old terms.
- **Alternative:** the picket reads the terms live.
- **Why:** at 24:1 a passage spans real hours. A player who launched and logged off made no
  decision about the new price. A works fee is pinned at booking for the same reason (Q-48-53).

**Q-50-11 — When the new terms begin.**
- **Proposed default:** at the clock's completion, for passages filed after it. No operation and no
  waiting period: a front's public clock, visible from the first day, is its notice.
- **Alternative:** the terms begin 28 IC days after completion.
- **Why:** Q-45-15 keeps the operation notice for a change in what a player or organization holds.
  This changes a price for everyone, as EX.8's weekly step does without notice. One caution: the
  Co-op, an NPC organization, does lose its de facto hold.

**Q-50-12 — What advances the clock, and its size.**
- **Proposed default:** as built: a fine, a seizure or a run each advance it by one, and a clean
  inspection does not. Six segments, one filled, provisional. The front is seeded at world start so
  `+clocks` shows it from the first day.
- **Alternative:** doc 11's four segments; or a clean, paid inspection also counts.
- **Why:** the code and doc 11 disagree on both points. With smuggling as the only cause, a hauler
  who runs unlicensed ore raises every hauler's costs, which is the conflict of interest §6.4
  wants on this route.

**Q-50-13 — Whose front it is.**
- **Proposed default:** the Republic's board, as doc 11 has it. Today it is on the Freeport
  Registry's.
- **Alternative:** leave it where it is.
- **Why:** the code opens it on the license issuer's faction because that organization was at hand.
  The outcome is the Republic's gain. Needs Q-50-2.

**Q-50-14 — Who is paid.**
- **Proposed default:** before the flip, as built: the contraband fine goes to the Freeport
  Registry, whose license it enforces. After it, fines and duty on the approach go to a Republic
  customs office, a new organization row in the lane (provisional capital).
- **Alternative:** Stickney Yards as the Republic's payee, with no new organization.
- **Why:** enforcement follows de facto control (Q-50-4). The owner may also want to rule on
  today's oddity, a Republic corvette collecting for the Freeport.

**Q-50-15 — Whether the picket's own findings change.**
- **Proposed default:** no. Under the Republic's hold the picket still fines when the pilot can pay
  and seizes when they cannot. The duty is the one change.
- **Alternative:** under the Republic's hold unlicensed ore is always seized, which is what the
  consequence text says.
- **Why:** one change at a time. The first cut's test is one price measured before and after.

**Q-50-16 — Whether it can be pushed back.**
- **Proposed default:** not in the first cut. The hold stands until a later unit gives the Freeport
  side a clock that flips it back, which is the engine's own model of opposition, or staff act.
- **Alternative:** an opposing clock on the Freeport's board now, advanced by licensed strategic
  ore clearing the picket.
- **Why:** the owner asked for a small first cut. Worlds are wiped between steps (Q-38-9). The
  causes of a counter-clock belong with the campaign work of §6.3.

### 6.3 The Isidis operation

**Q-50-17 — What Isidis is, in play.**
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

**Q-50-18 — What the operation is, in the engine.**
- **Proposed default:** both engines, each used as built. A `mush-campaign` operation is the frame:
  declared, bond held, notice, window, sides, stakes, one resolution record. The two fronts' clocks
  are the visible progress. Resolution commits the operation's canon record; a clock watching it
  completes and flips the control record. TacSim gains no campaign edge: it commits canon, and the
  campaign tick reads it.
- **Alternative:** fronts and clocks only, with no operation.
- **Why:** Q-45-15 already rules that a change of control is an operation with notice. Doc 11 has
  the Review's completion "declare an operation". The engine was built for this and nothing has
  used it.

**Q-50-19 — Who declares, and when.**
- **Proposed default:** a compiled arm on the port tick acts for the Republic. When the Concession
  Review fills it declares the operation from the Republic's treasury, bond held, naming the
  Freeport and Meridiani as defenders, with 28 IC days' notice and a 28-day window. No player or
  staff step. At 24:1 that is about 56 real hours end to end.
- **Alternative:** staff declare; or a player officer of a Republic House may.
- **Why:** the faction design's own test is "no faction-head bottleneck". Institutions are
  compiled arms today.

**Q-50-20 — What moves the Review before that.**
- **Proposed default:** the two causes of doc 11 the code can supply: a seized Freeport-flagged
  hauler at Kirkwood, which the picket already writes as canon, and a scheduled tick of one an IC
  month (about 30 real hours). The scheduled tick is a new typed cause, deferred since M25. The
  clock starts at two of six.
- **Alternative:** seizures only.
- **Why:** doc 11 §6 says these clocks are "what move when nobody is logged in".

**Q-50-21 — The stockpile objective.**
- **Proposed default:** a named vault at Isidis (the tank farm), a list of goods with target
  tonnages (provisional: propellant and provisions) and a deadline, the window's close. Progress is
  the vault's level, read on the port tick. "Threshold reached" is committed once as canon, which
  the operation's objective reads.
- **Alternative:** count deliveries as events, each haul's marker advancing a clock.
- **Why:** doc 11 words it "stockpiles reach threshold". A level can be fed by a haul job, by a
  sale into a standing bid or by the faction's own hull, so a player who took no job still counts.
  A level is safe across a restart. Events double count on a retry unless each carries a cause id.

**Q-50-22 — The budget, and who spends it.**
- **Proposed default:** the Freeport side's money is its backing organization's treasury (Occator,
  for the tank farm). A compiled arm on the procurement pattern, with a monthly cap counted from
  its own ledger rows and an agent object of its own, posts haul jobs from Occator's stock at Ceres
  to the tank farm, escort jobs for them, and a standing bid at Isidis. When the cap or the
  treasury runs out, posting stops. No money is minted for it.
- **Alternative:** staff post; or the faction posts a bid and no jobs.
- **Why:** procurement is the working precedent of an organization spending toward a goal with no
  player. A real budget makes interdiction a way to bleed it.

**Q-50-23 — Posting as an organization.**
- **Proposed default:** haul and escort posts gain `as=<org>` for a member holding a new
  capability, paid from the organization's account and shipped vault to vault. The NPC arm uses the
  same path.
- **Alternative:** only the NPC arm gets an organization path.
- **Why:** today only a character standing in a port can post, from their own locker to their own.
  A player House will want this the day after. It needs §4.3's NPC defect fixed or worked around.

**Q-50-24 — If no player takes the job.**
- **Proposed default:** after a provisional 7 IC days an untaken haul is carried by doc 48's NPC
  hauling house (EX.10) in a real hull that can be intercepted. The operation resolves with nobody
  connected.
- **Alternative:** an untaken job lapses, and the faction is simply short.
- **Why:** the faction design's test: delete every player, and the conflict must still advance.
  It depends on EX.10 landing.

**Q-50-25 — What a loss does.**
- **Proposed default:** cargo seized, wrecked or captured on the way never reaches the stockpile.
  It costs the hauler the bond, as today. When the hull flew for a side of the operation it also
  commits a canon record that advances the other side's clock by one. The haul gains the handling
  it lacks: on a wreck the reserved cargo becomes part of the wreck, and on a seizure the job fails
  at once.
- **Alternative:** money only.
- **Why:** doc 11 already names a seized Freeport-flagged hauler as a cause for the Review. The
  owner asked that "deliveries or losses affect the declared operation".

**Q-50-26 — Whether players may fight over a hauler in space.**
- **Proposed default:** inside a declared operation's window, hulls flying for its named sides are
  legal targets to each other in a bubble, read from the operation's legal-target list. Outside it
  today's rule stands: every carrier is friendly to every other. This answers doc 47's open call
  for this one case. **(expensive to reverse)**
- **Alternative A:** no change. Only the picket, the interdictor and raiders threaten a hauler.
- **Alternative B:** any carrier may fire on any other, the act recorded as evidence (§6.4).
- **Why:** an escort has nothing to guard against but NPCs today. Limiting it to a declared window
  leaves doc 45's absent-player rules as they are everywhere else.

**Q-50-27 — Work for players on the Republic's side.**
- **Proposed default:** in the first operation the Republic's arm posts a standing bid at Stickney
  for the same goods at a premium, and retrieval jobs on interdicted hulls. Both use job kinds that
  exist.
- **Alternative:** nothing for the Republic's side in the first operation.
- **Why:** two buyers for one cargo is a choice a trader can feel with no new job kind: every tonne
  sold to the Republic is one not landed at Isidis. A campaign with one side's work is a quest.

**Q-50-28 — How it is decided.**
- **Proposed default:** at the window's close. If the threshold record exists, the Concession is
  held and the Freeport keeps de facto administration. If it does not, the Republic assumes it.
  The declaring arm resolves; staff do not. No fight is required and none is forbidden.
- **Alternative:** a ground engagement at Isidis decides it, and the stockpile only weights it.
- **Why:** facility assaults and territorial warfare are out of scope by ruling. Supply is what
  docs 48 and 49 just built. A declared operation over a ground theater, settled by what was
  landed there, is the smallest honest reading of "a ground operation that changes control or
  access". The owner may prefer the alternative once assaults are in scope.

**Q-50-29 — What winning changes.**
- **Proposed default:** the de facto control record for Isidis, and through the schedule the
  import duty there: zero while the Freeport administers it, and a provisional 5% of reference
  value under the Republic, on a list of goods in the schedule, paid to Republic customs. The
  venue fees' and the berth's payee follows the holder as well.
- **Alternative:** the de jure record changes too, and with it who issues licenses there.
- **Why:** "whoever holds it decides whether Belt cargo lands untaxed." Untaxed is the only state
  today, so the prize needs the tax. The duty names goods and not origin because the survey found
  nothing on a lot that records where it came from (§8).

**Q-50-30 — Following it.**
- **Proposed default:** one player page, `+operation`, on one screen: who declared against whom;
  the stake in plain words, as what each outcome changes in the terms; the notice and the window
  in IC and real time; both clocks; the stockpile's level against its target; the jobs open for
  it. Past operations stay listed with their result. The job board tags an operation's jobs. A
  queued notice and IC mail go out at the declaration and at the resolution to every player with a
  hull on the lane (doc 45 Q-45-17's channel).
- **Alternative:** `+clocks`, `+chronicle` and the job board as they are.
- **Why:** the owner's words are "discover, influence, and follow". `@op/show` is shaped for staff.
  Nothing today joins the pieces.

**Q-50-31 — Whether it repeats.**
- **Proposed default:** yes, by the same shape. A held Concession sets the Review back to two of
  six, and it climbs again. An assumed one opens a Freeport front to win it back, sides swapped.
  Only the first operation is built first.
- **Alternative:** one operation a world.
- **Why:** worlds are wiped today, but the shape should cycle when they are not.

### 6.4 Law: jurisdiction, evidence, penalties and standing

**Q-50-32 — The first restricted good, and its port.**
- **Proposed default:** strategic ore at Ceres. Its sale on the Freeport Exchange is restricted:
  sold under the Registry's prospecting license or a registered claim, the two papers the picket
  already reads. It stays lawful to sell at Stickney and Luna.
- **Alternative:** space munitions at Stickney, under Republic law; or a mind core, which its own
  closed families already police.
- **Why:** the rule, the papers and the check exist. It closes a loop with §6.2: ore that cannot be
  sold on the Ceres board is run inward past the Kirkwood picket to Stickney, where the house bid
  is also higher (700 against 600 in the lane pack), and each run caught there advances Customs
  Assertion.

**Q-50-33 — Where legality lives.**
- **Proposed default:** in the terms schedule, per polity: a list of goods, each restricted or
  forbidden, with the paper that lifts a restriction. A venue's rule for a good is the law of its
  region's de facto holder. Catalog rows read it in place of the hard-coded `legal`.
- **Alternative:** per venue, on the catalog rows.
- **Why:** law follows control (Q-50-4), so when Kirkwood or Isidis changes hands, what is
  restricted there changes with it and no second table is kept. The market's own comment gives the
  test to the ruleset.

**Q-50-34 — Where it is enforced.**
- **Proposed default:** at both ends, differently. The public board refuses an unlicensed seller
  of a restricted good: the exchange checks papers, so the venue gate learns the good. A private
  offer between players goes through, and is an offense if it comes to light. A forbidden good
  could not be listed at all; none is yet.
- **Alternative A:** refuse everywhere. That is a rule with no crime in it.
- **Alternative B:** refuse nothing, and act only after the fact.
- **Why:** this gives the smuggler a reason to find a private buyer, the buyer a discount to ask
  for, and the licensed trader a clean path. An exchange that sold contraband openly would make
  the license worthless.

**Q-50-35 — What establishes an offense.**
- **Proposed default:** port security's standard, generalized. An authority's own inspection: the
  picket's manifest check, or a port audit (Q-50-36). A witness report backed by the witness's own
  record no more than one IC day old: a party to the trade, or one present at the venue. An
  adjudicated contest. A bare log never fines.
- **Alternative:** the trade tape proves every sale.
- **Why:** Q-S4 forbids the automatic omniscient fine, and Q-41-24 says "proven rows only".

**Q-50-36 — The port audit.**
- **Proposed default:** at docking in a port whose law restricts a good, the hold is audited with a
  chance from the schedule, moved by the pilot's standing there: provisional one in four at
  neutral, always in the hostile band, never when trusted. It is a seeded roll, so a story can pin
  it.
- **Alternative A:** every docking is audited.
- **Alternative B:** no port audit; only the picket and witnesses.
- **Why:** "if caught" needs a chance, or smuggling is either impossible or free. It gives
  standing its first mechanical reader.

**Q-50-37 — One record of proven offenses.**
- **Proposed default:** each domain keeps its evidence where it is: the site log, the customs
  rulings and the Registry's inspections stay as they are. A proven offense is written through one
  function into a record per actor and per polity, which is port security's own record with a
  jurisdiction and a closed list of kinds added. Penalties and the player's page read that record.
  The Roll's sanctions are listed on the page and not moved. **(expensive to reverse)** in its key,
  actor by polity.
- **Alternative:** one ledger that replaces the domain records.
- **Why:** closed families stay closed. Port security's record is already half of this. Three
  enforcers need one reader.

**Q-50-38 — Jurisdiction.**
- **Proposed default:** an offense is proven by one polity and binds only where that polity holds
  de facto. Other ports ignore it. Port security's constant list of ports becomes "the ports held
  by the polity that proved it". A fine is paid to the enforcing authority: the port's `authority`,
  or the customs office on an approach.
- **Alternative:** one record for the whole corridor.
- **Why:** it gives crime a geography: wanted at Stickney and clean at Ceres is a route choice. It
  follows Q-50-4. It also ends a Stickney fine being paid to the Freeport Registry.

**Q-50-39 — The penalties.**
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
- **Alternative:** fines only in the first cut.
- **Why:** the owner named all three. Each has a working precedent: port security's fine and
  quarantine, standing grants with a cause, and the Roll's 90-day bar.

**Q-50-40 — Seeing it and resolving it.**
- **Proposed default:** `+record`, one page for the actor. Each open matter shows what it is, which
  polity proved it and by what kind of evidence (never who reported), the fine and its grace in IC
  and real time, the restriction in force, and exactly what lifts it. `+debts/pay` settles, as
  today. A queued notice and IC mail go out when a matter opens.
- **Alternative:** `+debts` and `+standing` as they are.
- **Why:** the owner's words are "a restriction they can see and resolve".

**Q-50-41 — Contesting a matter.**
- **Proposed default:** no appeal in the first cut. A matter is proven or it is not written.
- **Alternative:** a contest window of 28 IC days before the fine posts, heard by the claim-contest
  machinery or by staff.
- **Why:** the proof standard is high by construction: an authority's own check or a witness's own
  record. An appeal needs an adjudicator, and the storyteller role is disconnected (Q-44-29).

**Q-50-42 — How standing is earned back.**
- **Proposed default:** no decay, as built. Paying a fine restores nothing. Lawful work for that
  polity earns standing: survey commissions do today, and delivering to its stockpile or escorting
  its hulls would.
- **Alternative:** decay toward neutral over a provisional 90 IC days.
- **Why:** grants are immutable and stamped with a cause by design. Decay is a hidden clock.
  Work for standing ties this part to §6.3.

**Q-50-43 — A witness's reward.**
- **Proposed default:** a witness whose report proves an offense is paid a share of the fine when
  it is collected (provisional 20%), and is never named to the offender.
- **Alternative:** no reward.
- **Why:** it gives a player an inspector's interest on the route. Q-38-20 left bounties for later;
  this is the smallest one. A false report is not possible when the report must carry the
  witness's own record. Betrayal stays legal, recorded and discoverable, as the faction design
  asks.

**Q-50-44 — Seized goods.**
- **Proposed default:** credited to the enforcing authority's vault and offered at its port, so
  matter is conserved.
- **Alternative:** destroyed, and counted in the matter census under a `seized` term.
- **Why:** today the picket's seizure debits the hold to nobody. Seized ore for sale at a discount
  is something a trader can find.

### 6.5 Order, size and proof

**Q-50-45 — Order.**
- **Proposed default:** first the model (Q-50-1 to Q-50-5) with the Kirkwood cut as its first
  reader. Beside it, law's first cut: one good, one port, the record and the page; the two share
  only the schedule. The Isidis operation last, because it needs the place, organization posting,
  loss handling, EX.10 and the bubble rule.
- **Alternative:** Isidis before law.
- **Why:** the owner called Kirkwood the small first cut. Law reuses port security. Isidis carries
  most of the new work.

**Q-50-46 — Sizes at 24:1.**
- **Proposed default:** a front moves over real days, not hours. Provisional: Customs Assertion at
  six segments; the Review's tick one an IC month; the operation's notice and window 28 IC days
  each. All tuned after the playtest.
- **Alternative:** the longest notice and window the engine allows, 84 IC days each (three and a
  half real days each).
- **Why:** a rate per IC day is felt per real hour (Q-48-15's lesson). The playtest will say
  whether 56 real hours is a weekend's campaign or too quick to notice.

**Q-50-47 — The acceptance stories.**
- **Proposed default:** three, each on production command paths at 24:1 with two accounts.
  - A hauler files the same run before and after the Kirkwood flip, and reads the duty and its
    reason.
  - A player who never joined arrives at Isidis after the operation, and reads who won from the
    terms.
  - A smuggler's private sale is reported by its buyer, fined, restricted, paid and cleared.
  
  And one run with nobody connected: both fronts move and the operation resolves.
- **Alternative:** unit tests a piece.
- **Why:** the owner stated the first two as milestones. The empty run is the "no faction-head
  bottleneck" test.

**Q-50-48 — The bot.**
- **Proposed default:** the mining bot learns the duty line and the `+record` page, and its
  sentence table gains each new sentence. No bot lane plays the operation in the first build.
- **Alternative:** a bot cohort hauls for the stockpile in the nightly.
- **Why:** docs 47 to 49 hold every unit to the sentence table. A campaign lane is clock-bound and
  slow, and doc 43's clock seam (TS.10) is not built.

**Q-50-49 — The other fronts.**
- **Proposed default:** out of scope. Registry Divergence, The Slowdown and the relay fronts keep
  their text outcomes.
- **Alternative:** give every shipped front a control outcome now.
- **Why:** The Slowdown's outcome ("throughput falls, prices spike") is a second schedule row once
  the model exists, and is better sized after the first three have been played.

## 7. Units of work (a sketch; cut only after the rulings)

Three chains. The names and the split are provisional.

**KC: control and the Kirkwood cut**
- KC.0: the polities as factions from content; the lane's `regions` block; the control read
  (Q-50-1 to Q-50-4).
- KC.1: the customs front on the Republic's board, seeded at world start, completing through the
  engine's path with a control flip (Q-50-12, Q-50-13).
- KC.2: the terms schedule; the transit duty in the customs ruling; pinning at filing; the payee
  (Q-50-5, Q-50-6, Q-50-10, Q-50-11, Q-50-14).
- KC.3: the quote lines and the reason clause; the first acceptance story (Q-50-8, Q-50-9).
- KC.4: the Co-op's enrollment share by control (Q-50-7).

**LW: law**
- LW.1: legality from the schedule; the venue gate learns the good; the board's refusal (Q-50-32
  to Q-50-34).
- LW.2: the record of proven offenses; jurisdiction by control; port security moved onto it
  (Q-50-37, Q-50-38).
- LW.3: the port audit; the witness report for a sale and its reward (Q-50-35, Q-50-36, Q-50-43).
- LW.4: the standing loss, the trading restriction, and `+record` (Q-50-39 to Q-50-42).
- LW.5: seized goods to a vault (Q-50-44).

**OP: the Isidis operation**
- OP.1: Isidis as a place (Q-50-17).
- OP.2: the scheduled tick as a typed cause; the Concession Review and Hold the Concession
  (Q-50-20).
- OP.3: posting as an organization; the haul's loss handling (Q-50-23, Q-50-25).
- OP.4: the stockpile and the Freeport arm's budget (Q-50-21, Q-50-22, Q-50-24).
- OP.5: the declaring arm, the operation frame and the resolution into control and the import duty
  (Q-50-18, Q-50-19, Q-50-28, Q-50-29).
- OP.6: legal targets in a bubble (Q-50-26).
- OP.7: `+operation`, the notices, the Republic's bid (Q-50-27, Q-50-30).
- OP.8: the empty run and the second acceptance story (Q-50-47).

**Edges with work in flight.** Doc 48's EX.9 owns the consumption arm and the lane's stock rows,
and EX.10 the NPC hauling house, which OP.4 leans on. Every KC and LW unit that adds a schedule
row bumps the lane, so those run one at a time, as doc 48's lane chain did.

## 8. What this survey did not check

- **Where a passage is priced before launch.** The lanes found the picket's ruling and no fee on a
  passage. Whether a plan or filing screen exists that could carry a duty estimate is not
  established (Q-50-8).
- **Whether a lot can prove its license.** The picket reads the pilot's papers and claim tenure,
  not the ore's provenance. A buyer of unlicensed ore may hold nothing that marks it (Q-50-35,
  Q-50-36).
- **Whether a lot records its origin.** "Belt cargo" is taken here as a list of goods (Q-50-29).
- **The trade tape's visibility** to a player who was not a party.
- **Whether a seeded roll is to hand** in the port tick for the audit.
- **The escort fact in code.** It is cited from doc 48's row and the module's header.
- **Doc 11 §9's own questions.** Their rulings were not found; doc 11 is Draft 0.1.
- **How `+territory` reads in TacSim today**, with no region seeded.
- **What EX.9 and EX.10 will have changed** by the time these units are cut: the faucet, the
  stipend and the hauling house are in flight.
- **What the dev boxes run.** Nothing here was tried on a live server.
