# Mining

Surface mining is the trade run's other trade: you take a Dray-P down onto a
mining site's pad, send its terrain vehicles out, learn where the ore lies, dig
it, and carry it home to sell at Ceres. This page walks the loop beat by beat.
Some of it is not built yet; those beats say "(not yet)".

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
(`+licence/buy`, `help coop`), the Co-op's yard refits a chartered Dray into a
Dray-P (`+coop/refit prospector`), and Occator leases terrain vehicles into its
bay at the Exchange Hall counter (`+tv/lease`, `help ships`). A lease needs a
licence in force or in its grace.

  +licence/buy [go]        the Freeport Registry's prospecting licence
  +coop/refit prospector   a Dray-P: a two-slot vehicle bay and a stern ramp
  +tv/lease [go]           lease a rig into a free bay slot

**2. Pick a body and a site.** `+chart` shows every site with the Registry's
survey of its province (the same for everyone) and its tenure: commons, open,
or held by an org.

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
  +tv/<n> recover [go]     back up the ramp into the bay; from further off the
                           rig drives to the ramp first, in the same act
  +tv/<n> dump [go]        empty the hopper onto the site's spoil, for good
  +tv[/<n>]/map [site] [family]  the site map around the rig, or the whole site

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
longer, and the preview names both legs. A rig's battery recharges in the bay
and only falls outside it; a scan or a sounding that would leave it under 5 % is
refused (the charge home counts too: beat 7), but a recover is never
held to the 5 %: a rig whose charge covers the drive to the ramp always comes home.

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
flight, and the rig can always come home with `+tv/1 recover go`. Every map
shows the pits worked so far (`v`, and `V` once worked out).

  +tv/<n> mine [family] [cycles] [go]  mine the pocket the rig stands on
  +tv/<n> drill [family] [cycles] [go] the same
  +tv/<n> mine stop        end the run after the cycle in flight

**9. Hopper full, go home.** (not yet) Unloading a rig's hopper into the hold.
Until then a rig comes home with `+tv/1 recover go`: from anywhere on the site's
ground it drives to the ramp and climbs aboard as one act, even while the
instruments are dark, and it is refused only when its charge cannot cover that
drive. `+ship/lift ceres` lifts the Dray-P off the pad once every rig is in her
bay and every hopper is empty (`help ships`).

**10. Stake.** (not yet) Staking an open block and the Registry's claims.

**11. Leave it running.** (not yet) Fill plans, and awaiting a rig from the
order queue.

**12. Sell.** Ore sells only at Ceres, to the Exchange's and Occator's standing
bids. `+market/sell <listing>` shows what a sale would take and pay;
`+market/sell <listing>=all` sells what the bid takes and leaves the rest
aboard.

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

Unloading; staking and the Registry's claims; fill plans and awaiting a rig;
the ground chart of a whole body.

See also: `help ships`, `help coop`, `help travel`, `help trade`.
