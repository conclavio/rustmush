# Building with the agent

The building agent turns a brief into a **content pack** — the same typed records
staff write by hand (`help atlas`) — and nothing is built until you say go. You
ask for a site in a neighborhood that allows player building, the agent reads the
map, proposes the site, checks it against the validator, asks you what it cannot
decide, and hands you a proposal to read. `+build/go` makes it real: your site
stands at once, playable, marked pending review, and a reviewer's ticket goes to
the neighborhood's builders.

  +build/place <kind> [<where>]=<prose>   ask for a site — a park, a shop, a
                                          house, a garden — in the neighborhood
                                          you stand in, or one you name
  +build/show [<id>]               a proposal: its pack, findings and questions
  +build/answer <id>/<n>=<answer>  answer question n; the planning goes on when
                                   every question has an answer
  +build/feedback <id>=<text>      steer another round of planning
  +build/go <id>                   build it
  +build/withdraw <id>             drop a proposal and the request behind it
  +build/status                    your proposals and your standing

The game never blocks: `+build/place` reserves the ground and starts the
planning; you are told when the proposal is ready, or what it asks. A proposal
that does not validate says why (`+build/show` lists the findings) and
`+build/feedback` sends it round again. Each proposal has a cost cap; a run that
reaches it keeps what it has and stops.

## What a request reserves

A request holds three things while it is outstanding: one of your **request
slots** (two at a time by default; staff may set yours), one unit of the
neighborhood's allowance of player-built sites, and the **ground** — the
footprint nobody else may build on. Asking twice for the same site converges on
the first request rather than charging a second slot. A site that a reviewer
approves releases the slot; one they ask changes to stays live and keeps it.

## Review, strikes and standing

A reviewer may approve your site (it stays, the slot is released), ask for
changes (the site stays live; fix it with a new proposal), or **strike** it: the
site is retired and your strike count rises. At three strikes your cap is zero
until staff reset it. `+build/status` shows your slots, your strikes and every
request you have made; nothing on it expires.

Staff: `@atlas/cap <character>=<n>` sets a cap (and clears strikes),
`@atlas/strike <site>=<reason>` strikes a site, `@atlas/standing [<character>]`
reads a standing or the whole request book, and `@atlas/propose` runs the same
agent for a staff pack that applies live (`help atlas`). Every planning call is
metered and audited (`help llm`); a server with the LLM layer off says so.
