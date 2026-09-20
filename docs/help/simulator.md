# The simulator

The simulator centre is an out-of-character annexe off the Welcome Hall where
two sides fly stock tanks against each other on the proving ground. Nothing that
happens in it is part of the story: it is practice, and a wrecked tank is put
back at spec afterwards.

`help cockpit` is the other half of this topic — what you type once you are in
a pod.

## Getting there

From Limbo, `ooc` takes you to the Welcome Hall; `simulator` takes you east down
the corridor into the Simulator Lobby. From the lobby, `red` and `blue` are the
two team rooms, and `lobby` (or `out`) is the way back.

The lobby has a console bolted to the counter: `look Simulator Selector` says
the same thing this topic does, in fewer words.

## Claiming a slot

  SIM TEAM red|blue   claim one pilot slot on a side. One slot per account,
                      four a side. Whoever claims a side first leads it.
  SIM LEAVE           give your slot up; the last one to leave closes the match.
  SIM START           lock the pods and open boarding. A side's first pilot, or
                      staff, with at least one pilot a side.
  SIM ABORT           call the match off (any pilot in it, or staff).
  SIM STATUS          the centre: its state, its sides, its slots and its pods.
                      The bare word SIM is the same thing.
  SIM AI [n]          how many units a side fields when the machines fill in.
                      A side's first pilot or staff, before the match starts;
                      the bare word reads the number.
  SIM UNIT [type]     which unit type your own slot fields. The bare word lists
                      what this server can field and what you are flying.

A slot belongs to your **account**, not to your character, so an alt cannot take
a second one. All of these work in the lobby, the two team rooms and the pods,
and nowhere else.

## Choosing what you fly

Each pod has a tank of its own, and that is what you fly unless you say
otherwise. `SIM UNIT` says otherwise:

  SIM UNIT                    Fieldable units: biped_mk1, line_tank,
                              line_tank_eccm, line_tank_ecm, line_tank_laser,
                              line_tank_thermal. Yours: line_tank.
  SIM UNIT biped_mk1          Slot red 1 will field biped_mk1.

It changes **your own** slot and only during selection, and everyone in the
centre reads the line. `SIM UNIT line_tank` — the pod's own type — puts you back
to the default. The pod's tank is actually refitted when `SIM START` runs, and
it is put back to the pod's own type the moment your slot goes — you giving it
up, the boarding deadline dropping you, the centre going idle after the match,
or a staff `@arena/reset`. Every `SIM START` also puts back any pod that nobody
has chosen for, so nobody ever inherits anybody else's choice.

Once the match has started the board stops printing anybody's choice: by then
the tanks have been refitted, and what another machine is remains something
your own sensors have to tell you.

What the shipped server can field:

  line_tank           the stock tank: a 105 mm cannon, a coaxial machine gun
                      and a Mk1 sight.
  line_tank_laser     the same hull with a beam weapon where the cannon sat,
                      keeping the coaxial machine gun. It carries no cannon at
                      all, and the beam makes heat.
  line_tank_thermal   the stock tank with a thermal sight: it picks a stopped
                      machine out of cover that the eye would lose, at shorter
                      range, not through a canopy, and slower to tell apart.
  line_tank_ecm       the stock tank carrying a jamming pod, which shortens
                      everyone's sensor reach near it, friends included.
  line_tank_eccm      the stock tank carrying a counter-jamming filter, which
                      takes some of that back.
  biped_mk1           a two-legged machine: taller, quicker to turn and slower
                      flat out, and its sight sits nine metres up.

Three things worth knowing before you pick one:

* a bare `FIRE` fires the **cannon** whenever the machine has one, and
  otherwise the first weapon it carries. The laser tank carries no cannon, so
  its bare `FIRE` is its beam; name the weapon (`FIRE laser`, `FIRE
  machine_gun`) and there is nothing to remember.
* a legged machine still pays the tracked machine's price for crossing a cell.
  That is a known gap, not a feature of the chassis.
* the machines that fill the unclaimed pods always fly the pod's own tank.

## Fighting the machines

On a server with the AI fill switched on, the tanks of the pods nobody claimed
are flown by the simulator itself, and **one pilot is enough to start a match**.
Each side fields as many units as the larger side has claimed slots — so alone
you get a one against one, and two of you on red get two machines on blue. `SIM
AI 3` asks for a bigger game; it cannot go below what a side has already
claimed, or above the four pods a side — and if more pilots claim afterwards the
number follows them up, because both sides always field the same count. The
board says which it is:

  AI fill: each side fields 2; machines take the unclaimed pods.

A machine patrols, closes on what it can see, holds the range its gun likes and
shoots. Hurt badly enough it breaks off for its own entry zone — but it is not
out of the match: cornered on the way home by something it can see and shoot it
stops and fights back — with an empty magazine it keeps driving — and after
twenty quiet seconds with nothing hostile in sight it turns round and comes back
in, once. A tank whose pilot left does the same, so a disconnection leaves your
side a teammate rather than a parked target.

With the fill switched off, `SIM AI` answers `AI fill is off on this server.`
and both sides need a pilot of their own, as before.

If you claim into a centre that is still showing the last match's result, your
claim clears the board first and puts the tanks back at muster. You do not need
staff for a rematch.

## Boarding

`SIM START` answers `Match N is boarding.` Walk into your own team room (`red`
or `blue`) and type:

  BOARD               climb into the pod your slot holds and take its tank's
                      cockpit.
  EXIT                climb back out.

The match starts the moment every claimed slot is seated, or when the boarding
window runs out and the unseated slots are dropped. Everyone in the centre reads
`Match N is running. Good hunting.`

**`EXIT` once the match is running is final.** The seat, its control lease and
your focus are released, the tank goes to its autopilot for the rest of the
match, and you cannot board again. Losing your connection does the same thing
once the link-dead grace runs out.

## The match, and afterwards

While it runs, the cockpit is the whole of it: `help cockpit`.

It ends when one side has nothing left that can fight, when **neither** side has
a round left to fire (a draw: `ended — draw: both sides are out of ammunition`),
when the match's time runs out, or when somebody aborts it. Running dry on your
own does not lose you the match — the other side has to be dry too.
Everyone in the centre reads
`Match N is over (red). Its units are checked in.`, the hatches open, and the
tanks stand where they stopped, damaged as they ended — that is the match's own
evidence, and `SIM STATUS` keeps showing the result until somebody starts the
next match. Staff can read the full record with `@arena/status` and the tanks
with `@unit/show`.

The next `SIM TEAM` clears the board, puts all eight tanks back at spec, back at
muster and back to their own types, and opens selection again.

## When there is no simulator

`This server has no simulator centre.` means the world was seeded before the
centre existed; staff have to take the content upgrade.

`The TacSim layer is off on this server` means the arena and its tanks were
never applied; staff have to turn the layer on and restart. The centre's rooms
and pods are there either way, which is why the refusal says so out loud.

See also: `help cockpit`, `help chargen` (the ratings the shooting reads),
`help getting-started`.
