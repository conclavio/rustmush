# Shadowrun

The game runs on **Shadowrun 5th Edition** (SR5). Your runner is described by a
sheet you build in chargen and roll against in play. The dice are a **pool of
d6**: you gather Attribute + Skill (+ modifiers) in dice and roll them all at
once — every die showing a **5 or 6 is a hit**. Success means meeting the test's
**threshold** (default 1 hit); the difference is your **net hits**. Two SR5
signatures shape every roll:

- **Limit** — a test can never count more hits than its Limit (Physical, Mental,
  or Social, derived from your attributes, or a gear/weapon limit). Excess hits
  are wasted.
- **Glitch** — if *more than half* of the dice you rolled come up **1**, something
  goes wrong (a **glitch**) even on a success; a glitch with **zero hits** is a
  **critical glitch** — a disaster.

Only one game system runs on a given server. This is a Shadowrun server, so the
World of Darkness commands are not present.

## Your sheet

  +sheet [<target>]    Show your character sheet (or a runner you can examine).
                       Staff-only fields are hidden from players.

Staff view and edit with `@sheet <target>` and `@sheet/set
<target>/<field>=<value>` (map entries use `<field>:<key>=<value>`). Editing an
attribute or Essence automatically recomputes your Limits, Condition Monitors,
and Initiative.

## Rolling — +roll

  +roll <attr>+<skill>[+/-mod]     Roll a d6 hit pool from your sheet.
  +roll <n>                        Roll a raw pool of n dice.

The pool resolves against your sheet: `+roll agility+pistols` gathers your
Agility and your Pistols skill. Skills you lack default at **-1** (if the skill
allows defaulting); a skill covered by a **skill group** you bought uses the
group rating. Your **wound modifier** (-1 per three boxes of damage) and the
**sensible Limit** for the test apply automatically.

  Switches:
    /edge      Spend a point of Edge to PUSH THE LIMIT: add your Edge rating in
               dice, ignore the Limit for this test, and apply the Rule of Six
               (each 6 explodes into another die). Deducts one Edge.
    /edge+     Just add your Edge rating in dice (no Limit waiver, no explosion).
    /priv      Result to you and in-room staff only.
    /nowound   Ignore the wound modifier.

  Inline options (after the pool):
    threshold <n> / vs <n>   Hits needed to succeed (default 1).
    limit <physical|mental|social|n|none>   Override the Limit.
    mod <n>                   A flat dice modifier.
    spec <name>               Apply a specialization you have (+2 dice).
    contested <target>=<pool> Resolve an opposed test against a target.
    extended <n>              Accumulate hits toward n across repeated rolls.

  +roll/last [<target>]      Show recent rolls (staff may read another's).

Every roll is announced to the room, kept in your personal roll log, and pushed
to the `sr.roll` panel with the full per-die record.

## Damage & Condition Monitors

You have two Condition Monitors: **Physical** (8 + half your Body) and **Stun**
(8 + half your Willpower). Stun that overflows its track becomes Physical
damage; Physical past its track is lethal **overflow** (you die past Body boxes).

  +condition [<target>]   (alias +cm)   Show both monitors and the wound modifier.
  +damage <target>=<n>P|<n>S   Apply damage (staff). P = Physical, S = Stun.
  +heal <target>=<n>P|<n>S     Clear boxes (staff).

## Initiative

  +init [<target>]     Show Initiative (Reaction + Intuition + initiative dice).
  +init/roll           Roll your Initiative dice and announce your score.

## Magic & Resonance

Spells, adept powers, and complex forms are tracked on your sheet and **rolled**,
not automated.

  +magic / +resonance [<target>]   Show your magical / Resonance status.
  +spells [<target>]               List known spells.
  +spell [<name>]                  Learn a spell (chargen/staff) or list yours.
  +cast <spell> [force <f>] [threshold <n>]
        Roll Magic + Spellcasting (Limit = Force), then resist DRAIN
        (Willpower + your tradition's Drain attribute). Overcasting (Force above
        your Magic) makes the Drain Physical.
  +powers [<target>]               List adept Power Points and powers.
  +complexforms [<target>]         List complex forms.
  +thread <form> [level <l>]       Thread a complex form (Resonance + Software,
        Limit = Level), then resist FADING (Willpower + Resonance).

## Character generation — the priority system

Chargen uses the classic SR5 **priority** method: assign A, B, C, D, E (each used
once) across five categories — **Metatype, Attributes, Magic, Skills,
Resources** — then spend the allotments plus 25 **Karma** on finishing touches.

  +chargen                Begin (or resume) chargen.
  +chargen/wizard         Guided, step-by-step creation.
  +chargen/priority metatype=A attributes=B magic=C skills=D resources=E
                          Assign the five priorities.
  +stat <field>[:<key>]=<value>
                          Allocate a trait (e.g. +stat body=5, +stat
                          active_skills:pistols=4, +stat metatype=ork).
  +spec <skill>=<specialization>   Record a specialization (+2 dice when it applies).
  +chargen/status         Show the live budget.
  +chargen/submit         Finalize and file for gamemaster approval.

The engine enforces the allotments, the metatype attribute maxima (only one
attribute may sit at its maximum at creation), and the skill caps, and refuses to
submit an unbalanced sheet. On approval a gamemaster stamps your sheet; derived
stats (Limits, Condition Monitors, Initiative, Essence → Magic) are computed for
you.

## Advancement — Karma

Karma is the advancement currency (the XP of Shadowrun).

  +karma [<target>]                       Show balance and history.
  +karma/spend <field>[:<key>]=<new>      File a costed raise for approval
        (e.g. +karma/spend body=5, +karma/spend active_skills:pistols=5,
        +karma/spend spell:Fireball=1). Applied atomically on approval.
  +karma/award <target>=<n>               Grant Karma (gamemaster).

Costs follow the SR5 table: raising an attribute costs new-rating x5, an active
skill new-rating x2, a skill group new-rating x5, and learning a spell or complex
form is a flat cost.
