# Character generation

Your character is described by a **pilot sheet**, and it is short: three
identity fields and three ratings.

| Section | Fields |
|---|---|
| Identity | `name`, `callsign`, `concept` — free text |
| Ratings | `gunnery`, `piloting`, `sensors` — 1 to 5 |
| Standing | `certifications`, `reputation` — staff set these, not you |

There is no health track, no money and no experience on the sheet. A tank's
condition lives on the tank, not on you; money is the ledger's.

## The three commands

  +chargen                   Start (or resume) chargen: you get an unapproved
                             sheet and the live budget.
  +stat <field>=<value>      Set one field, e.g. `+stat gunnery=3`,
                             `+stat callsign=Ladder`.
  +chargen/status            The budget, and anything still blocking submission.
  +chargen/submit            File the sheet for staff approval.

`+sheet` shows a sheet at any time.

## The budget

Every rating starts at **1**, free. You have **six** more dots to spend across
the three ratings, one point a dot, and no rating may go past **5**. So `5/2/1`
and `3/3/3` both spend it exactly, and `5/5/1` is refused at the keystroke
rather than at submission — `+stat` tells you the moment a spend would overrun.

`name` and `callsign` must both carry something before you can submit.

Which rating matters where: **gunnery** is the one the shot resolution reads —
a higher rating is a tighter cone and a better chance on every round you fire.
**piloting** and **sensors** are on the sheet and the simulator does not read
them yet, so spend with that in mind.

## Submitting

`+chargen/submit` files a job on the `pilot-chargen` queue. While it is pending
the sheet is **locked** — you cannot re-open it with `+stat` until a storyteller
approves or bounces it — because an unlocked sheet could be edited past the
budget and then rubber-stamped by the job already open. On approval the sheet is
stamped and it is yours.

## When chargen is required to play (`chargen_required`)

Some servers gate play behind an approved sheet. On those, a brand-new character
is placed in a quiet **chargen area** and greeted with a pointer to `+chargen`.
Until a storyteller approves you, you are held to a small command set — chargen,
`look`, `say`/`pose`, `help`, `who`, and the channel-management verbs — and
everything else (walking onto the grid, speaking on a channel, building) waits.
The moment your `pilot-chargen` job is approved the restriction lifts and you
are moved to the game's live start room. If your server does not require chargen
(the default), you play immediately and can build your sheet whenever you like.

See also: `help simulator` (what the ratings are for), `help jobs` (how approval
queues work), `help getting-started`.
