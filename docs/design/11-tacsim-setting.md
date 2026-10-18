# 11 — TacSim setting: the corridor and its factions

**Draft 0.1 — 2026-09-05.** The world-building companion to doc 10. Doc 10 fixes the *shape* of the world (§1–§11: premise, scales, tone, comparative advantage, the six organization concepts, layered control, risk zones, the Mars–Phobos–Ceres corridor); this document fills it with **names, history, actors and motives**, and maps every one of them onto the records the engine already has — `mush-faction` factions, edges, control records, fronts and clocks; `mush-org` organizations with faction linkage; the cut-11 seed pack. Nothing here changes a locked decision; where a fiction choice leans on one, the decision is cited.

Three framing choices (owner, 2026-09-05) govern everything below: **old power vs. rising power** (Earth/Luna a hegemon past its peak, Mars converting autonomy into sovereignty, the Belt a loose freeport arrangement nobody fully governs); the corridor's conflict is **a Mars polity against a Ceres polity over the Ceres→Phobos flow**; and **mixed texture** — governments sound institutional, corporations sound like brands, mercenary units sound martial.

Companion docs: 10 (foundations), 10a/10b (E7), `docs/scenes_factions_knowledge.md` §3 (faction registry, edges, control, fronts/clocks, the four design tests), `docs/help/faction.md`.

---

## 1. The system in 2350, in one paragraph

Two hundred years of settlement — and a century and a half of Retreat before that — have produced three powers that need each other and trust each other less every year. **The Concord** — Earth and Luna — has the people, the money, the biotech and the only real navy, and can no longer afford to use the navy. **The Republic of Mars** has the heavy industry and the shipyards, won its autonomy forty years ago without quite winning a war, and is now turning autonomy into sovereignty one customs post at a time. **The Freeport of Ceres** has the water, the propellant, the Belt's bank and the Belt's claims registry, and has no army at all: it hires one. Everyone's ships are slow except the warships, and the warships are so expensive to run that nobody can keep them everywhere. Into that gap go corporations, cooperatives, mercenaries, salvors and haulers — which is to say, the players.

## 2. History that is still load-bearing

Only the past that explains a present-day record is recorded here. Dates are CE.

| When | What | Why it matters now |
|---|---|---|
| 2040s–2080s | **The Retreat.** Sea-level rise, storm intensification, subsidence and salinization make the coastal megacities uninsurable, then unfinanceable, then abandoned, defended or rebuilt inland; wet-bulb heat closes parts of the tropics seasonally; the agricultural belts migrate poleward; migration in the hundreds of millions; sovereign defaults; the global supply web fractures. The wealthy withdraw into **Enclaves** — highland, high-latitude, inland, offshore, orbital, lunar — privately governed, climate-controlled, defended. Emissions crash with the economy: the collapse does what policy could not. Population peaks near 10.5 billion. | Earth's political geography is inland and highland. The drowned coasts are still the Concord's **Adaptation Fund**. **"Dry money"** is what the Belt calls Concord finance. Every name in the corridor is a mixture, because everyone moved. |
| 2060s | **The Triage.** During the evacuation of a drowning megacity, an Enclave-operated automated system allocates rescue by projected economic value. The leaked logs are the age's most famous document; the death toll makes *automated authority* the crime of the era. Placeless by design. | **No machine decides who is saved.** The Charter's Warrant (2268) is the Triage's law, and *unwarranted* is still the corridor's word for the unforgivable. |
| 2080s–2200s | **The Interval** ("the Long Retreat"). No functioning global order: the blocs contract inland and contest water, land and migrants; no state funds big science; the leading-edge fabs and the supply web are gone and compute is scarce — capability froze with it, and the Enclaves' minds become heirlooms. Knowledge is kept; scale is lost. The **pockets** keep the curve moving: the lunar and orbital habitats (closed-loop ecology, regolith industry, compact fusion — later fusion propulsion and the entangled-pair foundry), the highland biotech Enclaves (longevity, crops, medicine), the inland fission-and-hydro basins (heavy industry), the archives. The **Registry** is founded at Luna (c. 2140) as the notary of last resort for the habitats' claims among themselves and the first Belt ventures. Enclave consortia launch **arks** to Mars (2150s–2200s) as retreats and hedges, staffed by contract crews who stay when the owners' money stops. Population troughs near 6 billion. | Luna holds the space-technology lineage: the only pair foundry (doc 15), the yards, the seat. The Registry is older than the Concord, and the Concord knows it. Mars's founding population is ark *crews*, not ark owners. **Habitat discipline** — automation legible, overridable, maintainable; it does, it does not decide — is the Concord's engineering culture. Compute scarcity is why automation is excellent and people are decisive. |
| 2200s–2260s | **The Convergence.** Warming slows — the emissions crashed a century earlier — and the new coastlines settle; the inland blocs reconstitute and rebuild trade; population recovery begins. The Enclaves, small, hated and dependent on the blocs for people and food, need legitimacy; the blocs need the Enclaves' capital and the habitats' technology, and above all **orbital access**, which only Luna has. Three decades of negotiation. | The Concord's founding bargain trades legitimacy for capability. The Belt's view that the Concord is "the Enclaves with a flag" is not wrong; the Concord's view that it is the reconciliation is not wrong either. |
| **2268** | **The Concordat.** The **Concord of Earth and Luna**: a treaty polity pooling five functions — the Adaptation Fund and climate engineering, migration, orbital access and the fleet, the Registry, currency clearing — and nothing else; internal governance stays with the blocs, former Enclaves among them. Its **Charter** settles four things: Luna's seat at Shackleton and the Registry's primacy; amnesty for the Enclaves in exchange for their capital and archives; the **outbound mandate** — the solar system as the answer to a wrecked Earth; and automation under human command, as the **Warrant** (no automated system may own, contract, command, adjudicate or sign; every automated act has a liable principal) and the **Compact** (training scale rationed; the rebuilt fabs chartered in the Earth basins; frontier-class **minds** few, licensed, located and audited — they advise, offices are human). | "Nobody in the Belt cares which city." The Concord's legitimacy rests on delivering the outbound dividend, which is exactly what it can no longer afford. The class polarization of the Retreat sits inside the Concord's own legislature. The Chancellor's generation remembers the signing. |
| 2270s–2310s | **The Outbound.** The renaissance: fusion torch hulls from the lunar yards; the Malapert pair network (doc 15); mass migration to Mars, absorbing the arks; the Belt opened under the Registry; Concord Fleet built; population recovery accelerates on Enclave biotech. | The navy is the Outbound's navy — the *Halcyon* is elderly because it was built for a push that ended. The Republic and the Freeport are the Outbound's children. The renaissance generation is dying out, and the Concord is in its late-institutional phase: rich, tired, overextended. |
| 2271–2281 | **The Dry Decade.** The Outbound's first failure, three years after the Concordat: Mars's volatile imports fail to keep pace with settlement growth. The Concord, unable to guarantee supply, grants Belt carriers a tariff-free treaty enclave at Isidis Planitia — **the Isidis Concession** — administered by the carriers' own port authority, in return for guaranteed water and propellant deliveries. | The Concession is a Concord-era treaty the Republic *inherited* — the new Concord's first humiliation, inherited with it. The Freeport holds de jure administration of a patch of Mars. |
| 2308–2312 | **The Martian Settlement.** A decade of tax refusal and the Phobos yard strike; two years of intermittent shooting in orbit; the Concord discovers that a blockade of Mars costs more propellant than the tariffs it would recover. Negotiated autonomy: the **Republic of Mars** is recognized, inherits Concord-era treaties and a share of Concord-era debt. | The Republic is legally bound to the Concession and financially bound to Luna. Both are resented. |
| 2312 | As part of the Settlement, the Concord recognizes the **Freeport of Ceres** as licensing authority for Ceres and its associated claims, on condition its Registry stays mirrored with Luna's. | The Freeport's sovereignty is real but *derived*; its Registry is the leash. |
| 2320s | The Belt Carriers' Cooperative builds **Kirkwood Depot** at the inner edge of the Belt on the Ceres–Mars lane, financed by Tycho Mutual, licensed by the Freeport. | The corridor's transfer node exists, and three parties have paper on it. |
| | **As built (doc 18 IC2):** Kirkwood's orbit is the owner's open question Q-18-36; the trade run's lane pack only places Kirkwood on the Ceres–Mars chord at the epoch with a placeholder period, and this row's wording waits on the ruling. | |
| 2340s | The Republic makes Phobos a real yard (**Stickney Yards**), commissions torch-drive patrol cutters for the **Orbital Guard**, and from 2347 stations customs pickets at Kirkwood — "customs assertion". | The interception geometry of the corridor is a policy, not an accident. |
| late 2349 | The Assembly at Coprates votes to **review** the Isidis Concession. The Freeport's Port Council answers by extending Meridiani Security's contract and stockpiling at Isidis. | World start. The Concession Review clock is at 2 of 6. |

**Population, order of magnitude (tunable):** Earth peaked near 10.5 billion in the 2080s and troughed near 6 billion in the 2190s; in 2350, Earth ~9 billion, Luna ~30 million, Mars ~60 million, the Belt ~3 million of whom ~800,000 are on or around Ceres, Venus/Mercury/Titan each under a million.

**Money.** The corridor settles in the **credit (cr)**, cleared through Luna. The Republic has floated pricing Phobos yard-time in a Martian **mark** — a political act, not a monetary one, and a later story rather than a seed-pack fact (§9 q1).

**The argument.** The Retreat, the Interval, the Concordat, the Warrant and the Compact are argued in [`proposals/tacsim-history-retreat-and-concordat.md`](proposals/tacsim-history-retreat-and-concordat.md) (accepted 2026-09-12, twelve rulings).

## 3. The three governments

Each is a `mush-faction` faction: charter, typed resource pools, NPC principals, edges, control records, fronts. Each must act with **zero PCs** (`scenes_factions_knowledge.md` §3.7 test 1) — the principals and the fronts below are what move when no player is logged in.

### 3.1 The Concord of Earth and Luna — "the Concord"

*Institutional; younger than its institutions; solvent; overextended.* Seat: the Concord Executive at **Shackleton**, Luna. Legislature on Earth; nobody in the Belt cares which city.

- **Exports** finance, biotech, electronics, high-end manufacturing, naval hulls. **Needs** bulk metals and volatiles it no longer produces at scale — which is why it cannot let the Belt slip.
- **Instruments of power:** the **Registry** (Luna's claims registry, mirrored at Ceres by treaty — the legal root of every Belt claim); **Concord Fleet**, of which the **Belt Squadron** — one elderly *Halcyon*-class cruiser and escorts — sits on station at Ceres to "guarantee the Registry"; debt (Ceres Clearing and the Republic both owe Luna); Tycho Mutual, the Luna insurer that underwrites most of what moves in the corridor; the **Adaptation Fund**, the coasts' standing bill, which is why the treasury is large and unavailable; and the **Compact** — the chartered fabs in the Earth basins and the licences every mind runs under, the Earth blocs' counterweight to Luna's foundry and Registry.
- **Wants:** the Registry intact, the Concession *not* to become a war it would be treaty-bound to notice, and Mars kept dependent. It will spend money and words freely and hulls almost never.
- **Principals:** **Admiral Perpetua Vance**, Belt Squadron, Ceres — the Concord's only face in the corridor, tired, correct, with orders that permit far less than her cruiser could do. **Registrar Ines Solano**, Luna — custodian of the Registry; speaks for the Concord on claims and licences. **Chancellor Idris Halloran-Mbeki**, Shackleton — offstage; appears as a statement, never a scene.
- **Resource pools:** `treasury` (cr, large), `fleet_days` (Belt Squadron endurance — the thing Vance actually rations), `registry_standing` (how much of the Belt still honours Luna's paper), `adaptation_bill` (the coasts' standing draw on `treasury` — large, committed, unavailable).
- **Corridor control records:** Earth/Luna all four kinds. **Economic** control at Ceres (via Ceres Clearing's debt and Tycho Mutual) and at Phobos (via Kasei Metals' debt). A **covert** presence at Ceres through the Squadron's intelligence section.

### 3.2 The Republic of Mars — "the Republic"

*Institutional; young; industrial; sovereignty-hungry.* Seat: the **Assembly at Coprates**. Head of government: the **First Minister**. Founded by the crews of the Enclave arks (§2), not their owners: its militias became an army and its strikes became a state, and it wants its own fabs as badly as its own water.

- **Exports** structural alloys, composites, industrial parts, munitions, hulls, refits, yard time. **Needs** volatiles and propellant (Ceres), strategic elements (Vesta/Pallas), electronics (Concord). Its war-making capacity depends on water it does not own.
- **Instruments of power:** the **Territorial Force** (ground; grew out of settlement militias, so its officers are political), the **Orbital Guard** (small; torch-drive *Vigil*-class cutters based at Phobos; the corridor's interceptors), **Stickney Yards** (a state-adjacent concern — the Republic holds a golden share), the **Office of Territorial Security** (OTS; intelligence; covert principal), and the power to *not buy* — it is the Freeport's biggest customer.
- **Wants:** the Concession ended and Isidis administered from Coprates; customs authority over the Kirkwood approach; its Phobos refinery out from under Belt debt; a fleet; a fab of its own, which the Compact forbids. It has the army and the cutters, and lacks the water, the money, and the legal case.
- **Principals:** **First Minister Ottilie Marquand-Reyes**, Coprates — a Settlement-generation politician who wants sovereignty finished in her term. **Commodore Teodor Bask**, Orbital Guard, Phobos — the hawk; believes one seized hauler ends the argument. **Colonel Sunniva Okoro**, Territorial Force, Isidis sector — commands the ground forces around the Concession; professional, does not want the operation she is planning. **Director Anselm Kettering**, OTS — covert principal; runs assets inside the Concession and the Co-op.
- **Resource pools:** `treasury` (cr, strained), `propellant_reserve` (the war-limiting quantity), `yard_time` (drydock-days at Stickney), `assembly_mandate` (political capital for the Review).
- **Corridor control records:** Coprates and the Martian core, all four kinds. Phobos: **de jure**, **de facto** (Orbital Guard), **economic** *shared* with Ceres Clearing (the Kasei debt). Kirkwood approach: **de facto** contested (pickets). The Isidis Concession: **de jure** claimant (contested with the Freeport), **covert** via OTS.
- **Edges:** `ChapterOf` none (sovereign). Stickney Yards `SubordinateTo` the Republic (golden share). Meridiani Security holds a Republic operating licence — which is not an edge, and the fact that it is *not* is the scandal (§4.4).

### 3.3 The Freeport of Ceres — "the Freeport"

*Institutional in name, mercantile in practice; rich; unarmed.* Governed by the **Port Council** (station councils plus registered claim-holders, so Occator Water has a seat and a half). Chief executive: the **Portreeve**.

- **Exports** water, propellant, logistics, finance, the Registry's Belt mirror. **Needs** everything manufactured — alloys, parts, electronics, hulls — and a security guarantee it has to rent.
- **Instruments of power:** the **Port Watch** (customs cutters, unarmed or nearly), the **Registry** (Belt mirror — issues claims, licences and export permits; the Freeport's sovereignty is exercised as *paperwork*), **Ceres Clearing** (the Belt's bank; holds Republic and Martian corporate debt), the **Isidis Concession** (treaty administration of a patch of Mars — its market access and its leverage), and the contracts it publishes: the Freeport is the corridor's largest *employer* of players.
- **Wants:** the Concession kept, the Kirkwood lane open and untaxed, the Registry mirror loosened enough to escape Luna without provoking Luna. It hires rather than commands, and knows exactly what that costs.
- **Principals:** **Portreeve Halvard Ndiaye** — former Occator executive; the Council's man, not the Belt's. **Factor Beatrix Oyelaran** — the Isidis Concession Factor, the Freeport's officer on Mars, and the defender of record when the operation is declared. **Registrar Tomasz Wen** — keeps the Belt mirror; quietly the most powerful person on Ceres, since he decides what a claim *is*.
- **Resource pools:** `treasury` (cr, large), `licences` (issuable claims/permits — the Freeport's real currency), `security_retainer` (contracted force-days), `concord_goodwill`.
- **Corridor control records:** Ceres station and docks: **de jure**, **de facto**; **covert** *held by the Tether*, not the Freeport (§4.6); **economic** Occator + Ceres Clearing. Occator claims: **de jure** (Registry), **economic** Occator. Kirkwood Depot: **de jure** (licence). The Isidis Concession: **de jure** claimant (treaty), **de facto** *via Meridiani Security under contract*, **economic** Occator (the tank farm) + Ceres Clearing.
- **Edges:** the Freeport `ContractedTo` Meridiani Security (time-boxed, linked to the retainer obligation). The Belt Carriers' Cooperative `ChapterOf` the Freeport (a Council seat). Occator Water is *not* an edge; it is a seat and a half.

### 3.4 The relationship matrix (public stance / private stance)

| From ↓ To → | the Concord | the Republic | the Freeport |
|---|---|---|---|
| **the Concord** | — | cordial / *contain* | guarantor / *keep the Registry; don't die for the Concession* |
| **the Republic** | friendly / *resents debt and the Squadron* | — | trading partner with a treaty dispute / *hostile: end the Concession* |
| **the Freeport** | ally / *wants out from under the Registry* | best customer / *wary; hedges with the Concord* | — |

Private stances feed principals' prompts and clock advancement; public stances are what `+faction` shows.

## 4. The corporations, cooperatives and companies

Each is a `mush-org` organization with real accounts (doc 10 S7A: **funded, never infinite** — capitalization, revenue, subsidy and loss budgets are seed-pack numbers), scoped divisions, and faction linkage where stated. NPC principals speak for them.

### 4.1 Occator Water & Propellant — "Occator"

*Brand-like; the Belt's largest concern.* Extraction and electrolysis at Occator crater, Ceres; owns the Isidis tank farm. Controlled by the Freeport's founding families; the Portreeve is an alumnus. **Provides:** the corridor's deposit and extractor drones, baseline volatile and propellant supply, survey commissions and hauling contracts at mediocre rates. **Principal:** **Ludmila Osei-Ferrante**, chief executive — regards the Republic as a customer and the Concession as her asset. Linked to the Freeport (a Council seat); **economic** control at Ceres, Occator claims and Isidis.

### 4.2 Stickney Yards — "the Yard"

*Brand-like; state-adjacent.* Phobos's drydocks; builds *Vigil* cutters, *Hod* dropships, *Anvil* mechs and *Cairn* GCVs for the Republic and, for a price, for anyone the Republic licenses. Sells **yard time** (a reservation, not a cargo — T4). **Principal:** **Yardmaster Ruaridh Tanaka** — a strike leader in 2309, an oligarch in 2350; wants the Yard busy and the Guard buying. `SubordinateTo` the Republic via the golden share.

### 4.3 Kasei Metals — "Kasei"

*Brand-like; Martian; indebted.* The corridor's **one refinery**, at Phobos: metals → structural alloys (`alloy_smelt@1`), volatiles → propellant. Built with Ceres Clearing money; the debt gives the Freeport's bank **economic** control of a Republic strategic asset, and the Republic knows it. **Principal:** **Managing Director Priya Castellanos** — serves two masters and is precise about which one she is serving on any given day.

**As built (doc 18 IC8):** in the trade run (lane pack v3) Kasei posts Stickney's reaction-mass ask and standing bids for metal concentrate and ice; its ice intake serves Phobos's volatiles demand, not the propellant line.

### 4.4 Meridiani Security Group — "Meridiani", "MSG"

*Martial-corporate; the seed's NPC mercenary command.* Founded on Mars by Territorial Force veterans; holds a Republic operating licence; **currently under retainer to the Freeport, defending the Isidis Concession against the Republic** — a scandal in Coprates, a business decision at Meridiani. Fields second-hand *Anvil-B* mechs and *Cairn* GCVs — the same hardware the Territorial Force will bring, because both were built at Stickney. **Provides:** the Concession's de facto security, baseline escort and facility-defence contracts, and the template every player mercenary command charters against. **Principal:** **Colonel (ret.) Ivo Marchetti-Nakamura** — will fight the Republic on contract and drink with its colonels after; expects the licence revocation and has priced it in. `ContractedTo` the Freeport; **de facto** control at Isidis.

### 4.5 The Belt Carriers' Cooperative — "the Co-op"

*Cooperative; the corridor's hauling line.* Owns and runs **Kirkwood Depot** and a fleet of *Dray*-class bulk haulers; publishes the baseline hauling contracts that keep the corridor's water moving when no player will. `ChapterOf` the Freeport (a Council seat). **Principal:** **Chair Dagny Holm** — a public face, and knows it. **The Co-op is the public, joinable, entirely real front for the Tether** (§4.6): the `FrontFor` edge is covert.

**As built (doc 18 IC6, IC13):** the Co-op runs the **First Captain programme** through the **Co-op First Captain Trust**, the programme's own institution: a new captain charters a Trust-titled *Dray* (a lease, a 25,000.00cr stake, 25% of trading margin to the Trust) at the Registry desk in Ceres, may buy her out at the programme's declared price (`+coop/buyout`), and a charter left a week without a departure goes back to the Trust's pool.

### 4.6 The Tether

*The covert one.* The Belt haulers' and dockworkers' association — everyone who has worked a cargo line, hence the name. Semi-legal; holds **covert** and much of the real **de facto** control of Ceres docks and crew rosters, and has people at Kirkwood and inside the Concession's dock labour. The Freeport pretends it is a guild; the Republic's OTS is trying to buy it; the Concord's Registry does not know it exists as a faction. **Principal:** **"Uncle" Matthias Kalu** — covert; a Co-op dispatcher on paper. **Fronts:** the Slowdown (§6). `FrontFor`: the Co-op → the Tether, covert by default. Membership visibility defaults to `covert`; the design's evidence-shedding rule applies (meetings are witnessable, rosters leak through Co-op paperwork).

### 4.7 Tycho Mutual Assurance & Salvage — "Tycho"

*Brand-like; Luna; the recovery provider (T7, Q7A).* A mutual insurer and salvor with agents at Ceres, Kirkwood, Phobos and Isidis. Underwrites hulls and cargo in the corridor; **owns the wrecks it pays out on**, which is where its loaner pool comes from; runs the **Re-float Program** — leases a *Dray* or a *Cairn* to a member who has lost everything, under `Encumbrance::Lease`, upkeep on the campaign cadence, no sale, no uninsured deployment. Replenishment is salvage and its own refit work orders at Stickney, never a mint (10b §3.11). Neutral by policy, Concord by domicile — a Concord economic presence in every port. **Principals:** **Underwriter-General Sarai Lindqvist**, Luna; **Assessor Pim Adeyemi**, Kirkwood — the person a ruined player actually meets.

### 4.8 Ceres Clearing

*Institutional-financial.* The Belt's bank and the Freeport's fiscal agent; the corridor's NPC market-maker with explicit cash, inventory, spread, volume and loss budgets (S7A). Holds Kasei's and the Republic's Belt debt; owes Luna. **Principal:** **Governor Eskil Varga**. **Economic** control at Ceres, Phobos (shared) and Isidis.

**As built (doc 18):** the trade-run slice does not use Clearing's market-maker role; its ports trade through their vendors' finite, receipted stock. The slice's Freeport Exchange is §4.12 (authored in 18a and the lane pack).

### 4.9 Lantern Haulage & Salvage — the seeded player corporation

*The one player corporation the seed pack needs (doc 10 §11).* A two-hull independent — one *Dray*, one tender — chartered at the Freeport, berthed at Kirkwood, neutral by necessity, willing to haul for anyone and salvage from everyone. Seeded with a charter, a small account, a Tycho policy, and no members: the first players walk into an existing legal person rather than a chargen screen. If the first players would rather charter their own, Lantern is the worked example of the corporation template (doc 10 §6).

### 4.10 Off-corridor stubs (strategic map only)

One line each, so the map has edges to grow into: **the Pallas Syndicate** (Vesta/Pallas metals and strategic elements; a mining combine that recognizes the Registry when convenient); **the Venus Aerostat Compact** (atmospheric chemistry, carbon feedstock; neutral, distant); **Mercury Solar Authority** (a Concord-chartered power and metals monopoly); **Titan Concession Company** (hydrocarbons; the outer system's only real settlement; months away).

**As built (doc 18 IC2):** Pallas has a body entry in the trade run's lane pack (the sky's bodies), off the slice's lane.

### 4.11 The Malapert Exchange — "Malapert"

*Institutional; extraterritorial; the one concern everyone pays.* Operates every quantum relay half in the system from **Malapert**, the Luna hub next door to Shackleton, under a Concord charter and the 2312 Settlement's extraterritoriality clause — Concord-era paper the Republic inherited and resents for the same reason it resents the Concession. Its own staff run the compounds at Phobos, Ceres and Kirkwood; nobody else touches a half. Sells **relay time** in four classes (Sovereign, Priority, Standard, Deferred — doc 15 §5.2), publishes a congestion board that overstates load, and can **throttle or suspend** any principal (doc 15 §5.3): arrears is the usual reason, Luna's displeasure the unusual one. **Principal:** **Postmaster Solveig Adebayo-Krishnan**, Malapert — chartered to be neutral, paid to be fast, answerable to Shackleton on the days it matters; the one person in the corridor who can make a government twenty minutes late. **Wants:** the monopoly intact (no private pairs), every account current, and never to be *seen* choosing a side. `SubordinateTo` the Concord via the charter; **de facto** control of every relay compound; **economic** control wherever a bill is due (doc 15 §4).

### 4.12 The Freeport Exchange — "the Exchange"

*Institutional; the Council's own window.* The Port Council's commodity floor, in the Exchange Hall at Ceres Freeport (18a C07): consigns the Belt claims' metal concentrate — Occator's drones and the smaller registered claim-holders who sit on the Council (§3.3, §4.1; doc 10 §11's Ceres export) — under the Exchange's mark, and takes the corridor's alloy intake under the name **Freeport Works**, the same house buying. Not a market-maker (that is Clearing's role, §4.8, which the trade run leaves unused): standing offers from finite, receipted stock, every line signed. **Provides:** the concentrate a new captain loads at Ceres and the alloy bid that pays for the return. **Principal:** **Factor Margit Søndergaard-Achebe** — a Council appointee who treats the Exchange's book as the Freeport's public ledger and the claim-holders' consignments as a trust; the officer of record and Warrant signatory for the Exchange's ask and the Works' bid, and for nothing of Occator's (one house, one signatory — the trade run's rule in the spirit of §2's principle). A Freeport institution: funded, never infinite (S7A).

**As built (doc 18 IC3, IC8; this entry added 2026-10-18 under doc 18 §10):** `freeport_exchange`, faction `freeport`, capitalized at 4,000,000 cr by one-shot treasury tranches — the slice's largest capital, a season's budget, because it is the return leg's buyer (IC3, `ea52634d`); its metals ask and Freeport Works' alloy bid are standing offers of the Ceres port pack signed by `npc.sondergaard` (IC8, `ee1d831f`; 18a C07).

## 5. Places, and who holds what

The corridor's regions as `mush-faction` regions with seeded `ControlRecord`s. Kinds: **J** de jure · **F** de facto · **C** covert · **E** economic. The point of the table is that almost every row has more than one holder — doc 10 §6's layered control is seeded, not hoped for.

| Region | Risk zone (S5A) | J | F | C | E | Notes |
|---|---|---|---|---|---|---|
| Coprates & the Martian core | core | Republic | Republic | Republic | Republic | The Republic's home; ground scenes, the Assembly. |
| **Phobos orbital domain & Stickney Yards** | core | Republic | Republic (Orbital Guard) | Republic (OTS) | Republic + Ceres Clearing (Kasei debt) | The yard, the refinery, the *Vigil* base. Where the interception is launched from. |
| **The Isidis Concession** (Mars ground theater) | frontier → **war zone** when the operation is declared | **Republic** *and* **Freeport** (contested — 10a Q14A's two claimants) | **Meridiani** for the Freeport | the Tether (dock labour); OTS (assets) | Occator (tank farm) + Ceres Clearing | Landing field, tank farm, tariff-free market, Factor's office. The ground op's objective is *administration*: whoever holds it decides whether Belt cargo lands untaxed. |
| **Kirkwood Depot & approach** (transfer node) | frontier / contested | Freeport (licence) | Co-op (runs it); **Republic pickets contest** | the Tether; OTS | Tycho (depot financing) | Propellant depot and relay at the Belt's inner edge. Replaces the fixture's "Cislunar Transfer Node" — cislunar is Earth–Moon, and this node is not. |
| The Ceres–Kirkwood–Phobos lane | frontier | — | — | — | — | The route. Deployed ships accept the risk of the space they are in (S5A). |
| **Ceres station & docks** (freeport region) | core | Freeport | Freeport (Port Watch) | **the Tether** | Occator + Ceres Clearing | The market (E6c), the Registry, the Council, the Squadron's berth. |
| **Occator claims** (extraction site) | core / frontier at the margins | Freeport (Registry) | Occator | — | Occator | Deposit, extractor drones, survey bands. An outer claim is seeded *contested* (a Pallas Syndicate paper claim) so claim-contest machinery has a live case. |
| Earth / Luna | core | Concord | Concord | Concord | Concord | Strategic map only in E7. |

**As built (doc 18 IC2):** Kirkwood is a seeded body of the trade run's lane pack, placed on the Ceres–Mars chord at the epoch with a placeholder period; its orbit is the owner's open question Q-18-36.

The five quantum relay halves the Malapert Exchange operates — the Luna hub, Phobos, Ceres, Kirkwood and Mercury — are assets inside these regions, not regions of their own; who holds what over each is tabulated in doc 15 §4.

## 6. Fronts and clocks at world start

These are what move when nobody is logged in (`scenes_factions_knowledge.md` §3.6; clocks advance only from typed causes; the pressure budget bounds them). Segment counts are seed values.

| Front (faction) | Clock | At start | Advances on | Completion |
|---|---|---|---|---|
| **The Concession Review** (Republic) | 6 | 2 | Assembly votes (scheduled tick, one per IC month); incidents at Isidis (scene outcomes); a seized Freeport-flagged hauler (canon event) | The Republic **declares an operation** (T6) to assume administration of the Concession — the corridor's ground engagement, with Meridiani the defender of record and the Freeport the eligible defender |
| **Customs Assertion** (Republic) | 4 | 1 | Each successful interdiction at Kirkwood (tactical fact `tactics.unit.captured` or a paid inspection) | Kirkwood approach flips to Republic **de facto**; Republic fees apply to the lane; the Co-op's rates rise |
| **Hold the Concession** (Freeport) | 6 | 2 | Meridiani reinforcements land; Occator stockpiles reach threshold; a hauler runs the pickets (`sim.journey.arrived` at Isidis) | Defences are pre-staged (S5A: defenders may pre-stage NPC forces and standing orders); the Belt Squadron issues a statement — the Concord is now *involved*, which is the Freeport's whole strategy |
| **Registry Divergence** (Freeport ↔ Concord) | 8 | 1 | Council votes; a claim honoured at Ceres and refused at Luna | The Belt mirror stops syncing; Concord paper becomes unenforceable in the Belt; the Squadron's orders change |
| **The Slowdown** (the Tether) | 4 | 0 | A crew stranded or unpaid (obligation default facts); an interdiction that kills a Co-op crew; either government "reforming" the docks | Ceres docks strike: throughput falls, prices spike, both governments discover who actually runs the port. Covert until it completes |
| **Terms of Service** (the Exchange) | 4 | 0 | A corridor principal's account past 30 IC days in arrears (cadence tick); a Luna request (a Concord front's consequence); an incident touching an Exchange compound or its staff (scene outcome, canon event) | The named principal is **suspended** (doc 15 §5.3): its traffic falls to light-lag fallback and a canon notice is published. Reversible on settlement; the audit line stays. |
| **A Half Afloat** (Concord) | 8 | 0 | Concord treasury spend (scheduled tick, one per IC month while funded); the Postmaster's consent (a principal decision); a Concord Fleet hull's refit completing at Luna (canon event) | The first **mobile relay half** launches aboard a Concord hull (doc 15 §7.2) — a canon event with a recap; the link registry gains a mobile node; the hull's host decides where instant comms go. |

Divergence between control kinds is itself a clock condition (the Concession's J/J/F split generates pressure on its own).

## 7. The units, named

Types are doc 10 §11's list; class names are fiction for the cut-11 rules pack, all built where the trade map says they would be.

| Doc 10 unit | Class | Built by | Fielded by | Notes |
|---|---|---|---|---|
| mining drones | *Occator-pattern* extractor drone | Occator (Ceres) | Occator; leased to claim-holders | Extraction at deposit bands; the thing that makes water. |
| hauler | ***Dray***-class bulk hauler | Stickney Yards | Co-op, Lantern, Tycho's loaner pool | Slow, efficient, unarmed. Cannot outrun a *Vigil*; can choose a burn that makes the intercept cost more propellant than the Guard wants to spend — the "fight or evade" decision is a propellant decision (T1). |
| | **As built (doc 18 IC2, IC4):** the trade run's *Dray* is 240 t dry with a 100 t, 120 m³ hold and six interior rooms; her drive is a tagged coast law whose season-dependence (Q-18-35) and laden law (Q-18-41) are open owner questions; she fits no core (Class 0, the drive ROM). | | | |
| patrol craft | ***Vigil***-class cutter | Stickney Yards | Orbital Guard | Torch drive: fast, expensive, **visible from a long way off** (doc 10 §3). The interceptor. |
| dropship | ***Hod***-class lander | Stickney Yards | Territorial Force; Meridiani; Co-op (civil variant) | Phobos → Isidis. Carries a *Cairn* or an *Anvil* and a platoon. |
| infantry | Territorial Force **rifle platoon**; Meridiani **security section** | — | as named | Infantry captures facilities (doc 10 §8): the Factor's office and the tank farm are infantry objectives. |
| tank | ***Cairn***-class ground combat vehicle | Stickney Yards | Territorial Force; Meridiani (*Cairn-B*); Tycho loaner pool | Cheaper on the Isidis flats. |
| mech | ***Anvil***-class mech | Stickney Yards | Territorial Force; Meridiani (*Anvil-B*, second-hand) | Excels in the broken ground around the Concession's perimeter. Both sides fight in Anvils, because there is one yard. |
| *(strategic map)* | ***Halcyon***-class cruiser | Concord (Luna) | Belt Squadron | One, old, at Ceres. Not fought in E7; its *existence* is a clock condition. |

## 8. What each faction offers players

Contracts create group play (doc 10 §7); every faction above publishes them.

- **The Freeport / Occator / the Co-op** hire: hauling Ceres → Kirkwood → Isidis, escort through the pickets, survey commissions on the outer claims, facility defence at Isidis. The corridor's biggest employer; pays in credits and licences.
- **The Republic / Stickney** hire: customs escort, salvage of interdicted hulls, refit and repair orders, "consultancy" for the Territorial Force around Isidis; and, once the operation is declared, open crew calls for the assault (T14). Pays in credits, yard time, and operating licences — the licence Meridiani is about to lose.
- **Meridiani** hires mercenary units and proxy seats (T10) for the Concession's defence; the template for player mercenary commands.
- **Tycho** insures, salvages, and re-floats the ruined; publishes salvage-rights contracts on every wreck the corridor produces.
- **The Tether** recruits — covertly — anyone who works the docks, and pays in information and favours rather than credits.
- **The Concord** publishes nothing and is watched by everyone; Vance's squadron is a clock condition players learn to read.

## 9. Questions for the owner (recommended defaults marked)

1. **Currency.** **A (recommended):** one settlement currency, the credit, for the whole corridor in E7; the Martian mark is a later political story. **B:** seed both, Phobos yard-time priced in marks from day one.
2. **The Concession's de jure split.** **A (recommended):** both the Republic and the Freeport are seeded as de jure claimants (10a Q14A's two-governments case, and the ground op's *legal* stakes). **B:** the Freeport alone is de jure by treaty; the Republic is a de facto claimant only — cleaner, less interesting.
3. **Who intercepts.** **A (recommended):** the Republic's Orbital Guard interdicts Freeport-bound haulers at Kirkwood (customs assertion) — the state is the aggressor in space, the Freeport the aggressor by proxy on the ground. **B:** Freeport-hired privateers raid Republic-contracted haulers — mirror image.
4. **The seeded player corporation.** **A (recommended):** seed Lantern Haulage & Salvage with no members, as a legal person the first players can occupy or ignore. **B:** seed no player organization; the first players charter their own against the template.
5. **Recovery provider.** **A (recommended):** Tycho Mutual's Re-float Program — neutral, Concord-domiciled, replenished by salvage. **B:** a Republic materiel program (loaners are Republic property, restrictions are political).
6. **Where this document lives.** **A (recommended):** `docs/design/11-tacsim-setting.md`, next to doc 10, since the seed pack is its consumer. **B:** a new `docs/setting/` tree, one file per faction, for a content wiki that outgrows the design series.
7. **Names.** Every proper noun here is a proposal. Answer `names ok`, or list replacements.

## 10. What the seed pack takes from this document

For cut 11 (10b §3.11), in the order the pack would declare them: the eight regions and their control records (§5); three government factions with charters, pools, principals and edges (§3); ten organizations and associations with capitalization and linkage (§4); seven fronts and their clocks (§6); one deposit with bands and a contested outer claim (§5, Occator); one refinery and its recipe (§4.3); Kirkwood Depot as the route's transfer node; eight unit classes (§7); the five relay halves and who holds them (doc 15 §4); and the twenty named principals — each an `@faction/principal` NPC with a prompt drawn from its faction's private stance (§3.4) at its clearance, never from ground truth.
