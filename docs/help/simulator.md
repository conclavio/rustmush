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

A slot belongs to your **account**, not to your character, so an alt cannot take
a second one. All of these work in the lobby, the two team rooms and the pods,
and nowhere else.

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

It ends when one side has nothing left that can fight, when the match's time
runs out, or when somebody aborts it. Everyone in the centre reads
`Match N is over (red). Its units are checked in.`, the hatches open, and the
tanks stand where they stopped, damaged as they ended — that is the match's own
evidence, and `SIM STATUS` keeps showing the result until somebody starts the
next match. Staff can read the full record with `@arena/status` and the tanks
with `@unit/show`.

The next `SIM TEAM` clears the board, puts all eight tanks back at spec and back
at muster, and opens selection again.

## When there is no simulator

`This server has no simulator centre.` means the world was seeded before the
centre existed; staff have to take the content upgrade.

`The TacSim layer is off on this server` means the arena and its tanks were
never applied; staff have to turn the layer on and restart. The centre's rooms
and pods are there either way, which is why the refusal says so out loud.

See also: `help cockpit`, `help chargen` (the ratings the shooting reads),
`help getting-started`.
