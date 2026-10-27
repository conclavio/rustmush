# Mining

Surface mining is the trade run's other trade: you take a Dray-P down onto a
mining site's pad, send its terrain vehicles out, learn where the ore lies, dig
it, and carry it home to sell at Ceres. This page walks the loop beat by beat.

## Two rules every rig act follows

**The go rule.** A bare act shows what it would do and commits nothing: what it
reads or moves, how long it takes and what it leaves in the battery. Add `go` and
it does exactly that. `+tv/1 scan` is a preview; `+tv/1 scan go` is the scan.

**The slot rule.** A Dray-P carries up to two rigs, one in each bay slot, and
the slot number is the rig's name: `+tv/2 scan` names rig 2. Without a number
an act addresses the rig you named last, or your only rig.

Landing is at the Registry's lettered sites this phase: Freeport Commons, Kerwan
Floor, Ahuna Flank, Juling Shadow and Occator Rim on Ceres, and Rheasilvia
Commons on Vesta (`help travel`).

## The loop

**1. Kit up.** At Ceres Freeport the Freeport Registry licenses prospectors
(`+licence/buy`, `help coop`), the Co-op's yard refits a chartered or owned Dray
into a Dray-P (`+coop/refit prospector`), and Occator leases terrain vehicles
into its bay at the Exchange Hall counter (`+tv/lease`, `help ships`). A lease
needs a licence in force or in its grace. After buying out your Dray, your title lets you
refit her and lease rigs on the same terms. Return any rigs before the buyout;
once title has passed, you can lease them again.

  +licence/buy [go]        the Freeport Registry's prospecting licence
  +licence                 your licence: in force, in its grace, or lapsed
  +coop/refit prospector   a Dray-P: a two-slot vehicle bay and a stern ramp
  +tv/lease [go]           lease a rig into a free bay slot

**2. Pick a body and a site.** `+chart` shows every site with the Registry's
survey of its province (the same for everyone) and its tenure: commons, open,
or held by an org.

`+chart/ground` lists the sites of the body you are on (or the one you name) by
letter: each one's survey, its tenure, the passage there now for your hull and
your own best reading of it. `+chart/ground map` draws that body flat, 56 by 28,
washed by the survey of one kind of ore (`.` barren, `-` lean, `+` fair, `*`
rich), with the port and the sites by letter.

  +chart/ground [<body>]   a body's sites: survey, tenure, passage, your best
  +chart/ground map [<body>] [<family>]  the body flat, one kind of ore's wash

**3. Hop and settle.** `+ship/plan <site>` and `+ship/undock <site>` fly you
down. Nothing sells reaction mass or provisions on a pad, so the departure holds
back the leg home: it is refused unless the tank and the locker cover the
passage out and the worst passage home. The crew's days on the pad are charged
when she lifts (`help ships`).

**4. On the ground.** `+tv/1 launch go` sends rig 1 down the stern ramp (300 IC
s). `+tv` says where each rig stands: its block and the block's tenure, its
field, its footing, and what you have read there — or "no readings here yet".
`+tv/map` draws the ground around the rig, 280 m a side at 10 m a cell, and
`+tv/map site` the whole site at 80 m a cell: the terrain, the hull, your rigs,
everyone else's, the public pits and your own readings of one kind of ore —
the kind you have read most of, or the one you name (`+tv/map volatiles`). Every
map carries this legend:

  @ you  H hull  o rig  , regolith  ^ rock  = ice  O crater  1–3 ore (t a cycle)  + other family  . barren  v/V pit
  # claim

A digit is your own reading of a pocket: 3 rich, 2 fair, 1 lean, and the
tonnes a drill cycle would give there. Nobody else's readings ever show on your
map, and yours never show on theirs.

  +tv[/<n>]                each rig: where it stands, its charge, its act
  +tv/<n> launch [go]      down the ramp onto the site
  +tv[/<n>]/map [site] [family]  the site map around the rig, or the whole site

**The quarry screen.** In the full-screen TUI (`help tui`) Alt+q opens the
quarry, and on the web its tab does the same: your rigs on the left, the map in
the middle and your readings and the log on the right. `s` `d` `x` `m` open the
box with `+tv/<n> scan`, `drive `, `sound` or `mine` for the rig you picked,
and nothing goes until you add `go`; `t` picks the next rig, `z` steps from the
drive view to the whole site to the body's ground chart, `l` shows another kind
of ore and `r` asks again. On the web a click on the map or on a reading opens
the box with the drive there.

**5. Scan.** `+tv/1 scan go` (or `+tv/1 survey go`) reads the nine fields around
the rig's own — each one the rig's 2.5 m mast can see over the ground — by
richness: 600 IC s and 1,000 cu of charge. How sharp it reads depends on the
footing under the rig: from rock each field reads one word, from regolith a band
of two words that always holds the truth, and from ice nothing comes back. The
preview names your footing and the nearest better one ("rock 60 m W"): where
you park is the prospecting decision.

**6. Sound.** `+tv/1 sound go` reads every 40 m pocket of the field the rig
stands in (when it stands in the field's middle; otherwise the 64 pockets around
it), family by family: 1,200 IC s and 2,000 cu. A sounding thumps through the
ground: every other party on the site hears it, as a bearing and a distance and
nothing more, at most once an IC hour from any one sounder on that site. `+tv/1 sound quiet go`
reads a quarter of that and nobody hears it, for the same charge and time.

  +tv/<n> scan [go]        read the nine fields around the rig
  +tv/<n> sound [quiet] [go]  read every pocket around the rig
  +tv/quiet [on|off]       stop (or start) hearing other rigs' soundings
  +tv/forget <site>|<body> [go]  strike your readings of a site, or of every
                           site on a body, from your chart

From the bay, a scan or a sounding goes down the ramp first: one act, 300 IC s
longer, and the preview names both legs.

**The reserve rule.** A rig's battery recharges in the bay and only falls outside
it. Every act that spends charge reserves it when it starts, and keeps back the
charge to drive home to the ramp from where it will leave the rig: an act that
would leave less than 5 % after that trip home is refused, and two acts never
spend one margin. A recover is held only to its own drive: a rig whose charge
covers the drive to the ramp always comes home. An unload spends nothing, and
the bay recharges the rig while it unloads.

**7. Drive.** `+tv/1 drive C117` drives rig 1 over the ground to a pocket; the
target can also be a field (`C-NW`, its middle), a block (`C`, its middle), a
site cell (`140,96`) or a bearing and a distance from where the rig stands
(`ne 300`). The preview names the distance, the time — a regolith cell is 5 IC
s, rock twice that — the charge it costs and the charge left after the trip
home: every drive, scan and sounding keeps back the charge to drive home to the
ramp, and one that would leave less than 5 % after it is refused. A drive to a
place the rig could not come back from is refused, and a way too winding to
hold in 32 waypoints is refused too: drive to a nearer point first. From the
bay the rig goes down the ramp first, one act 300 IC s longer. When it arrives
you hear where, with your reading of that pocket if you have one. No licence
and no claim is needed to drive anywhere on a site.

  +tv/<n> drive <target> [go]  drive to a pocket, a field, a block, a cell or a
                           bearing (C117, C-NW, C, 140,96, ne 300)

**8. Mine.** With rig 1 standing on a pocket, `+tv/1 mine` previews a run from
your own reading of that pocket: its class, each cycle's tonnes until the pocket
is drawn, the time, the hopper before and after, the hold's room after and the
charge after the run and after the trip home — and `+tv/1 mine go` starts
exactly that run. A cycle lifts the pocket's class in tonnes into the hopper —
lean 1 t, fair 2 t, rich 3 t, the last cycle the remainder — for 2,000 cu: 1,800
IC s on regolith or ice, 3,600 on rock. With no number the run goes until the
pocket is worked out, the hopper is full (12 t) or the hold's room is used up,
at most eight cycles; a number that does not fit is refused, and the refusal
names the number that does (`+tv/1 mine 3`). On a pocket holding two kinds of
ore, name the one you want (`+tv/1 mine ice`); without a name the run takes the
one with the most tonnes left, and the preview says what else is there.

A pocket you have no reading of is cored blind: "No reading here: the first
cycle will tell." It is one cycle, needs 3 t of room in the hopper and in the
hold whatever the pocket holds, and its core is your reading of the pocket from
then on. `+tv/1 mine stop` lets the cycle in flight finish and keep its ore; the
cycles after it are dropped at no cost. While a rig drills you may scan or sound
from it, preview its next drive, read the map and work your other rig.

Mining needs a prospecting licence in force or in its grace, and open ground or
the commons: "That block is under claim." refuses a block someone holds. Only
one rig works a pocket at a time: "Another rig is working that pocket." A
licence that lapses, or a claim filed, mid-run stops the run after the cycle in
flight, and the rig can always come home with `+tv/1 recover go`.

  +tv/<n> mine [family] [cycles] [go]  mine the pocket the rig stands on
  +tv/<n> drill [family] [cycles] [go] the same
  +tv/<n> mine stop        end the run after the cycle in flight

**9. Hopper full, go home.** `+tv/1 recover` previews the way home and what
follows it: from anywhere on the site's ground the rig drives to the ramp and
climbs aboard as one act, even while the instruments are dark, and it is refused
only when its charge cannot cover that drive. With ore in the hopper the preview
ends "then it unloads 12 t of metal ore, run-of-mine into the hold, 74 t of room
after" (a Dray-P's hold with nothing else in it), and once the rig is back in
the bay it unloads on its own: 600 IC s, the hopper into the hold, and you hear
"Rig 1 unloads 12 t of metal ore, run-of-mine into the Salt Moth's hold". The
hold always has room for what a hopper carries, because no cycle starts without
it. Should the hold have less (staff changed it), the rig unloads what fits and
says what stays in the hopper; a dump empties the rest.

`+tv/1 unload` previews the same unload by hand, for a rig in the bay with ore in
its hopper: every kind of ore the hold has room for, the first kind first.
`+tv/1 unload 3` unloads 3 t of the first kind only; a number past what the
hopper holds of it, or past what the hold has room for, is refused, and the
refusal names the most you can unload. Any act you file for the rig
before its own unload starts — a dump, a launch, your own `+tv/1 unload go` —
takes its place. `+ship/lift ceres` lifts the Dray-P off the pad once every rig is
in her bay and every hopper is empty (`help ships`).

If the rig cannot get home, or its queued unload is stuck after a change of helm,
ask staff for recovery. Staff can stop its work in place, bring it into its assigned
ship's bay, and unload its hopper into the docked ship's hold. Recovery keeps the ore;
the hold must have room for every kind of ore in the whole hopper. It does not renew
or return the rig's lease or refund its deposit. Wait for the recovery result before
starting more work, departing or returning the rig. If bringing it home is blocked,
staff receive the reason and can correct the problem or cancel the request.

  +tv/<n> recover [go]     back up the ramp into the bay; from further off the
                           rig drives to the ramp first, in the same act; a
                           loaded hopper then unloads into the hold
  +tv/<n> unload [<t>] [go]  the hopper into the hold, or <t> tonnes of the
                           first kind of ore
  +tv/<n> dump [go]        empty the hopper onto the site's spoil, for good

**10. Stake.** A block you have read yourself can be yours. Stand a rig in an
open block — not in the bay, not on the commons, not on a block someone holds —
and type `+claim/stake`. The preview lists your own readings of the block
("C-NW sounded, C117 cored"), how many soundings the site heard on the block
since it last opened before your first reading, any cooling term that binds you,
and the fee: "2000.00cr now and 2000.00cr a term (4000.00cr today)". A stake
needs a reading of your own made by hand since the block last opened — a scan, a
sounding, quiet or loud, or a single core will do; someone else's reading never
counts. Name a family to declare it on the Registry's books (`+claim/stake
metals`), and a rig's number when you have two (`+claim/stake 2`). Nothing is
filed until you type `go`.

`+claim/stake go` holds the fee out of your account as a public contract with the
Registry and files the claim over the whole block at once; the fee is the same
whatever the block holds. The Registry records it within a few seconds ("The
Freeport Registry records your claim on block C at Ahuna Flank until …") and the
fee is paid; if the Registry cannot take the stake, the fee comes back within the
hour. From then on only your rigs mine the block, its edge reads `#` on every
map, and `+registry` lists it. An account holds at most two claims.

A claim runs 168 IC days. `+claim/renew` extends it by another term, from
anywhere, once you have worked 50 t of the block this term (the preview says
"worked 32 of 50 t"); a claim that is not renewed lapses at its term's end and
the block reopens at once to anyone else. `+claim/release` gives a claim up now.
Either way the Registry takes no new stake on that block from you, or from any
character of your account, for one term after the lapse.

A rival who sounded the block loudly — the sounding the whole site heard —
after it last opened and before your first reading, within 30 IC days of your
stake, can take it with `+claim/contest <block>` from aboard a hull on the site:
they pay a stake and a term, and your last term fee comes back to you. A quiet
sounding is heard by nobody and gives no priority.

At Occator Rim the Pallas Syndicate's paper stands over the twelve outer blocks.
It bars nothing: stake and mine as anywhere. Work 100 t of a papered block in
your first term and your first renewal has the Registrar suspend the paper there;
work less and the paper stands, and your claim is not renewed. Occator holds the
four blocks around the pad.

  +claim/stake [<rig>] [<family>] [go]  stake the block your rig stands in
  +claim/renew [<site>] <block> [go]    renew your claim for another term
  +claim/release [<site>] <block> [go]  give a claim up now
  +claim/contest <block> [go]           take a claim on a sounding heard first
  +claim/list                           your claims and their worked tonnes
  +registry [<site>]                    every claim and paper at a site

**11. Leave it running.** A fill plan works a rig while you sleep aboard.
`+tv/1 plan C117,C118,D family=metal_ore` names pockets and whole blocks in the
order you want them worked and shows what the rig would do, step by step with
its IC hours; nothing is filed until you add `go`. A pocket is mined where it
stands (the drive there is the plan's); a block letter sounds the block's four
fields from each field's middle, nearest first, then mines every pocket of the
plan's ore that reads fair or better, nearest first. The ore is the one you name
with `family=`, or the best your first address reads; it never changes while the
plan runs. A plan names only places you have read yourself and may mine: a
pocket or block you have no reading of answers "You have no reading there.", a
block under someone else's claim "That block is under claim.", and the plan
needs your licence and the rig's lease like any act. A plan names at most 16
pockets and blocks and makes at most 64 steps.

Between steps the rig drives itself, recovers into the bay at every full hopper
and unloads into the hold, and comes home and parks when no cycle fits the
hold's unreserved room, its list is worked out, or its 288 IC hours have run;
the hours stop new steps, never one in flight. A plan fills one hold and never
starts again: you file each shift. It re-reads your licence, the rig's lease
and the block's claim at every step's start; if one has lapsed it stops mining,
comes home, unloads and parks, and says why. It waits while you are off the
helm or ashore, never past its 288 IC hours: then it ends where the rig stands.
If the rig's unload log is full, the unload waits and goes by itself once the
log frees, and the plan with it. Its soundings are loud, and everything it
reads is yours for every use but a stake: a claim needs a reading you made by
hand.

While a plan is live it holds its rig — no hand act and no lift until it ends.
`+tv/1 plan show` reads it back, step by step; `+tv/1 plan clear` ends it after
the step in flight (no `go`), and on a plan that has ended strikes it from the
rig's record. Both work from anywhere aboard, at the helm or not.

  +tv/<n> plan <pockets and blocks> [family=<good>] [go]  preview, then file a plan
  +tv/<n> plan show                                       read the rig's plan back
  +tv/<n> plan clear                                      end it, or strike an ended one

The captain's queue waits for the rigs: `+ship/order await tv all`, then
`+ship/order depart ceres` and `+ship/order sell metal_ore all`, flies the ore
home and sells it once every rig is parked — or after at most 288 IC hours
(`until=` sets fewer), when a rig still out holds the departure as ever.
`help orders` has the queue.

A rig left on the ground with nothing to do for 24 IC hours comes home and
parks by itself, unloading what it carries. One that cannot reach the ramp on
its charge is parked where it stands, and you are told once.

**While you were off watch.** Offline, a rig tells you one line a trip — its
hopper unloaded into the hold — and one summary when a plan ends (what it mined,
in how many cycles and trips, and the hold's state), plus a stop for a lapsed
right, an idle return, a stranded rig and any act refused; never a line a cycle
or a reading. They wait for you as "While you were off watch:" when you
reconnect. Connected, you hear every line as it happens.

**12. Sell.** Ore sells only at Ceres, to the Exchange's and Occator's standing
bids, and only from the hold: a rig's hopper never sells — unload it first.
`+market/sell <listing>` shows what a sale would take and pay and sells nothing;
`+market/sell <listing>=all` sells what the bid takes and leaves the rest aboard.
On a Co-op charter the Trust takes its share of an ore sale like any cargo's: a
quarter of the margin over half the bid (`help coop`).

  +market/sell <listing>[=all|=<t>]  at Ceres: sell ore from the hold to a bid

**The pits and the fog.** Every map shows the pits worked so far, the same for
everyone: `v` where someone has mined, `V` once the pocket is worked out. A pit
says only that someone mined there — never who, nor what is left. Everything
else on your map is your own reading, and "You have no reading there." is the
same words for a place you have never seen, a field you have not scanned and a
barren pocket you have not sounded.

## Survey commissions

Occator pays **500.00cr** for fresh detailed soundings of all **256 pockets** in
one commissioned block. `+survey` shows the board; `+survey <id>` shows the
terms and your progress. These four offers are shared by the whole world, and
each pays only once:

| Commission | Site | Block |
|---|---|---|
| `ahuna-a` | Ahuna Flank | A |
| `kerwan-f` | Kerwan Floor | F |
| `juling-k` | Juling Shadow | K |
| `occator-p` | Occator Rim | P |

`+survey/accept <id>` previews; add `go` to file. Occator reserves the reward
after your acceptance is recorded. Wait for the funding notice, then start new
soundings. You have **seven IC days** from acceptance to finish; another IC day
allows settlement, not more surveying. Your account may hold one unfinished
commission at a time, across all its characters.

Only successful `sound` and `sound quiet` acts begun strictly after the
acceptance time count, including a plan's soundings. Scan readings, mining
cores, another pilot's work and readings already in your chart do not count.
Sounding from the middle of each of the block's four fields covers it; quiet
soundings cover less ground and need more positions. Each pocket counts once.

Payment follows automatically when the funded commission is complete. Your
chart stays private, and commission progress survives forgetting a chart or
restarting the server. Neither makes a completed offer pay again.
`+survey/cancel <id>` previews giving up unfinished work; add `go` to cancel.
Recorded cancellation stops commission progress. Cancellation or expiry
returns the reserved reward to Occator, and the unfinished offer reopens once
the refund settles. Chart trading is still to come.

```text
+survey [<id>]                 the board, terms and your progress
+survey/accept <id> [go]        preview, then accept a commission
+survey/cancel <id> [go]        preview, then cancel unfinished work
```

## What a reading says

Every reading is your own: nobody else sees your chart, and you see nobody
else's. Richness is one of four words everywhere — barren, lean, fair, rich.

A scan's row names a field and, for each kind of ore it holds, its richness and
the number of ore pockets that word stands for: `C-NW fair–rich metals (5–16
pockets)`. A field is named by its block's letter and its corner of the block:
the sixteen blocks run A to D across the northern row to M to P across the
southern, and each holds four fields, NW, NE, SW and SE. A sounding names each
pocket by its block letter and its number in the block, counted from the
block's north-west corner: `C117 fair metals`. A pocket a sounding does not name
reads barren.

"You have no reading there." is all a place you have not read says — the same
words for a site you have never seen, a field you have not scanned and a barren
pocket you have not sounded. "You learn nothing new here." is a scan or a
sounding repeated from the very cell you last used it from when it would add
nothing; it costs nothing. The same instrument from any other cell always runs.

Soundings you hear name where they came from and how far, nothing else: "A
sounding thumps through the ground from the southeast, over 1 km off."

## Still to come

Occator's mill and its drones.

See also: `help ships`, `help coop`, `help travel`, `help trade`.
