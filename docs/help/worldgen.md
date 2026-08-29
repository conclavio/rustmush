# Worldgen

LLM-assisted room description. You describe a room in a sentence or two of
prose and get back a draft `@desc` written in the zone's house style and
consistent with its neighbours. The draft is yours to review — **nothing is
ever written to the room until you accept it** — and every call is metered and
audited. `@gen` needs the `BUILDER_LLM` power and the server's LLM layer
switched on (`[llm].enabled = true`; otherwise `@gen` just says so).

## Themes

A theme is the house style the generator writes to. It lives on a Zone Master
Object (so a whole neighbourhood shares it), on a single room (an override), or
on `#0` (the game-wide default), and rooms resolve it nearest-first up their
zone chain. Set fields with `@theme` (Builder+):

  @theme <zone>=<field>:<value>   Set one theme field on a zone/room/#0.
  @theme/show [<zone>]            Show the effective (resolved) theme (default: here).
  @theme/exemplar <zone>=<room>   Copy a room's accepted desc in as a gold-standard example.

Fields (list fields take comma-separated values):

  name          A label for the theme ("1920s waterfront").
  style_guide   Free prose: era, mood, materials, light and sound.
  vocabulary    Words/motifs to favour.
  banned        Words/anachronisms to avoid.
  pov_rules     POV and tense, e.g. "second person, present, no 'you see'".
  desc_length   short | medium | long.
  exemplars     One or more gold-standard descriptions (usually set via /exemplar).

## Generating and reviewing a room

  @gen here=<prose>       Draft a description for this room from your brief.

The game never blocks: you get an immediate "Generating..." line, and the draft
arrives a moment later (system output — the web client also gets a side-by-side
preview panel). The prompt carries the resolved theme, the room's real facts
(name and existing exits — the model never invents topology), and up to four
neighbouring rooms' *accepted* descriptions, so a run stays consistent with the
rooms around it. Then review:

  @gen/accept [fields]    Write the draft through the normal @desc/@set path.
                          Fields: any of `desc short details exits` (default: all).
  @gen/edit               Hand-tweak the draft description in a line editor.
  @gen/retry [guidance]   Regenerate; your guidance is appended and the rejected
                          draft is included as "avoid this".
  @gen/discard            Throw the draft away.

`details` become `look <keyword>` entries; `exits` proposes `@desc`s and transit
messages for the room's *existing* exits only. A draft persists, so it survives
a restart — you can accept it after a reboot.

## Whole buildings (`@gen/building`)

For a coherent multi-room space, describe the *whole building* at once. The
rooms and exits must already be dug — worldgen **describes; it never digs
topology** — and it works best when the building's rooms share a zone (so the
walk knows where the building ends). Stand in the entrance and:

  @gen/building <entrance>=<prose>   Generate a coherent whole building.

It runs in two reviewed stages:

1. **Brief** — one cheap call walks the building's interior room graph (BFS from
   the entrance, staying inside the zone; capped by
   `[worldgen].batch_max_rooms`, default 40) and drafts a shared *concept* +
   *palette* and a one-line concept per room. This is a reviewable draft — cheap
   to steer before any room is written.
2. **Rooms** — accepting the brief drafts one room at a time, each seeded with
   the brief *and its already-drafted neighbours*, so the rooms cohere. Drafts
   arrive as they complete; nothing is committed automatically.

Review the batch without moving:

  @gen/review [<building>]   List a building's rooms + their draft status.
  @gen/review next           Step to the next item (the brief, then each room).
  @gen/accept                Accept the current item (brief, or one room's desc).
  @gen/edit | /retry | /discard   As for a single room, on the current item.
  @gen/accept/all            Accept every drafted room in the building at once.

A budget trip mid-batch stops cleanly: the remaining rooms are marked
"needs attention" and nothing is auto-committed. The whole batch — brief and
room drafts — persists, so you can review and accept it after a reboot.

Start to finish (a small building already dug off the entrance):

  @gen/building here=A crumbling jazz-age speakeasy behind a barbershop; brass and gin.
  @gen/review here            (wait for "Brief ready")
  @gen/accept                 (accept the brief -> the room pass starts)
  @gen/review here            (watch "Drafted N/M" progress, then:)
  @gen/accept/all             (write every room's desc)

## Setting up and generating, start to finish

  @create Harbourside               (or use an existing Zone Master Object)
  @set Harbourside=ZONE_MASTER
  @theme Harbourside=style_guide:fog-bound Edwardian docks, gaslight and wet stone
  @theme Harbourside=banned:neon, plastic, chrome
  @dig/teleport Wharf               dig a room and step into it
  @zone here=Harbourside            put it in the zone (so it inherits the theme)
  @gen here=A long wooden wharf, tar and rope, a customs shed at the far end.
  @gen/accept                       write it, once you like the draft

## Staff: budgets and the kill switch (Wizard)

  @llm/budget             Today's LLM token/cost totals.
  @llm/dashboard          Fuller cost dashboard: budgets vs caps, top builders/
                          NPCs by spend, and recent calls (the terminal view of
                          the web LLM panel).
  @llm/log [<feature>]    Recent LLM calls (optionally one feature route).
  @llm/disable            Engage the global kill switch (stops all LLM calls).
  @llm/enable             Lift the global kill switch.

Every builder has a daily quota, and there are per-feature and server-wide daily
budgets on top; a call that would exceed any of them is refused before it is
made. Prompts are audited by hash and short summary only — never stored in full,
and the cost dashboard (`@llm/dashboard`, or the web **LLM** panel) shows
**aggregates only** — never any prompt text. See `help llm` for the full staff
LLM surface and `docs/ops.md` for the operator setup (providers, keys, budgets).
