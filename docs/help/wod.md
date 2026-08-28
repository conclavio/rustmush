# World of Darkness

The game runs on the Classic World of Darkness 20th Anniversary rules (the
Storyteller System — V20 / M20 / W20 core). Your character is described by a
sheet you build in chargen and roll against in play. This milestone ships the
base **Mortal** template; the Vampire, Mage, and Werewolf splats add their own
sheet sections and powers on top of it.

## Your sheet

  +sheet [<target>]  Show your character sheet (or a character you can
                     examine). Staff-only fields are hidden from players.

The sheet holds nine Attributes (Physical/Social/Mental), thirty Abilities
split into Talents / Skills / Knowledges, an open list of Backgrounds,
Willpower (a permanent rating and a temporary pool you spend), the seven-level
Health track, the three Virtues, Merits & Flaws, Experience, and your
concept / Nature / Demeanor.

## Rolling dice

  +roll <pool> [vs <diff>]

`<pool>` is a trait expression resolved against your sheet, or a plain number.
Examples:

  +roll strength+brawl              Attribute + Ability, difficulty 6.
  +roll dexterity+firearms vs 7     Set the difficulty (2–10, default 6).
  +roll wits+alertness+2            Add a flat modifier in the pool.
  +roll 5                           Roll a raw five-die pool (no sheet needed).

Trait names are matched by prefix, so `+roll dex+athl` works. The Storyteller
System: every die at or above the difficulty is a success, each **1** cancels a
success, and a roll with no successes and at least one 1 is a **botch**. A 10
counts double when you have a specialty.

Switches (put them on the verb, e.g. `+roll/wp ...`):

  /spec      A specialty applies — 10s count as two successes. Requires a
             specialty recorded (via +spec) on one of the pool's abilities.
  /wp        Spend a point of temporary Willpower for one automatic success
             (a Willpower-boosted roll can never botch).
  /priv      Show the result only to you and the storytellers in the room.
  /nowound   Ignore your wound penalty for this roll.

Inline options (in the argument, after the pool):

  vs <n> / diff <n>     Set the difficulty.
  mod <n>               Add a flat modifier.
  spec <name>           Apply a named specialty recorded on the ability.
  ext <n>               Extended roll: accumulate successes toward <n>.
  job <n>               Pin the result to job #n.
  contested <t>=<pool>  Resolve your roll against <t>'s pool; compare successes.

  +roll/last [<target>]  Replay your recent rolls (staff may read others').

Your wound penalty (from damage on your Health track) is applied to every pool
automatically, and a note is shown; use /nowound to skip it.

## Damage and health

  +health [<target>]         Show a Health track.
  +harm <target>=<n><type>   (staff) Apply damage; type is b/l/a for
                             bashing / lethal / aggravated, e.g. +harm Bob=3L.
  +heal <target>=<n><type>   (staff) Clear damage marks of a type.

Bashing fills the track from the top; once it is full, further bashing rolls
over into lethal. Lethal and aggravated sort ahead of bashing, and overflow
upgrades the least-severe mark. A full track is Incapacitated.

## Character generation

  +chargen                   Start (or resume) chargen. Creates your sheet.
  +chargen/attr physical=7 social=5 mental=3
                             Assign the 7/5/3 attribute priorities.
  +chargen/abil talents=13 skills=9 knowledges=5
                             Assign the 13/9/5 ability priorities.
  +stat <field>[:<key>]=<value>
                             Allocate a trait with live budget feedback, e.g.
                             +stat strength=3, +stat backgrounds:Resources=2,
                             +stat concept=Hard-boiled detective.
  +spec <ability>=<name>     Record a specialty on an ability.
  +chargen/status            Show your remaining budget.
  +chargen/submit            Finalize and file for staff approval.

Abilities cap at 3 during priority allocation; buy the 4th and 5th dots, and
anything beyond your priority pools, with your 15 freebie points (Attribute 5,
Ability 2, Background 1, Willpower 1, Virtue 2 per dot). When you submit, your
sheet is locked pending a storyteller's approval.

## Experience

  +xp [<target>]                     Show XP balance and spend history.
  +xp/spend <field>[:<key>]=<rating> File a costed raise for approval, e.g.
                                     +xp/spend brawl=3. The cost is deducted
                                     and the trait raised atomically when a
                                     storyteller approves the job.
  +xp/award <target>=<n>             (staff) Grant experience.

Costs follow the current-rating × multiplier table: Attribute ×4, Ability ×2
(a brand-new ability dot costs 3), Background ×3, Willpower ×1, Virtue ×2.

## Staff

  @sheet <target>                    Staff view of a sheet (shows hidden fields).
  @sheet/set <target>/<field>=<value>
                                     Set any field directly (validated, audited).
