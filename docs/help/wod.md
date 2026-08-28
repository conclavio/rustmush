# World of Darkness

The game runs on the Classic World of Darkness 20th Anniversary rules (the
Storyteller System — V20 / M20 / W20 core). Your character is described by a
sheet you build in chargen and roll against in play. Every character is exactly
one **template**: a plain **Mortal**, or one of the three splats — **Vampire**
(V20), **Mage** (M20), or **Werewolf** (W20) — each of which adds its own sheet
sections, powers, and commands on top of the shared base. You pick your template
during chargen (see below). The splat commands (`+disc`, `+cast`, `+shift`, …)
are always available, but each works only for a character of its splat and
politely refuses everyone else ("You are not Kindred.").

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
  +chargen/template <mortal|vampire|mage|werewolf>
                             Choose your template/splat. This adds that splat's
                             sheet sections (a vampire gets Disciplines / Blood
                             Pool / Generation; a mage Spheres / Arete; a Garou
                             Rage / Gnosis / Renown / Gifts). Mortal is the
                             default; switching wipes the previous splat's fields.
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

**House rule — uniform chargen pools.** This game uses the base oWoD pools for
every splat: 5 Background dots and (for mages) 5 Sphere dots. That is a
deliberate simplification of the by-the-book M20 mage, who receives 7
Backgrounds and 6 Spheres; use freebies or post-approval XP for the extra dots
a canon Mage build would start with.

## Experience

  +xp [<target>]                     Show XP balance and spend history.
  +xp/spend <field>[:<key>]=<rating> File a costed raise for approval, e.g.
                                     +xp/spend brawl=3. The cost is deducted
                                     and the trait raised atomically when a
                                     storyteller approves the job.
  +xp/award <target>=<n>             (staff) Grant experience.

Costs follow the current-rating × multiplier table: Attribute ×4, Ability ×2
(a brand-new ability dot costs 3), Background ×3, Willpower ×1, Virtue ×2.

## Splats

Powers are **sheet-tracked and pool-rolled**, the way the tabletop and most
MU*s run them: a power is stored on your sheet as a rating and rolled as a dice
pool, so there is no bespoke per-power automation — the mechanical result is a
Storyteller roll and the effect is adjudicated in play. Pick your splat with
`+chargen/template` (above); these commands then work for you and refuse anyone
of the wrong template.

**Vampire (V20)** — you are Kindred, of a Clan, at a Generation:

  +disc                      List your Disciplines.
  +disc/roll <disc>+<trait> [vs <diff>]
                             Build and roll a Discipline pool, e.g.
                             +disc/roll celerity+dexterity vs 6.
  +blood                     Show your Blood Pool.
  +blood/spend <n>           Spend blood; +blood/gain <n> to feed (capped by
                             your generation); +blood/heal [<n>] mends
                             bashing/lethal 1:1; +blood/buff <n> floods an
                             Attribute.
  +frenzy [vs <diff>]        Roll Self-Control/Instinct to resist frenzy;
                             +frenzy/fear rolls Courage against Rötschreck.
  +generation                Show your generation's Blood Pool / trait limits.
  +humanity                  Show your Humanity or Path rating.

**Mage (M20)** — you are Awakened, of a Tradition/Convention, with an Avatar:

  +sphere                    List your nine Spheres and your Arete.
  +sphere <name>=<0..5>      Rate a Sphere (it cannot exceed your Arete).
  +arete                     Show your Arete (magic rating and casting pool).
  +cast <description> [vs <diff>]
                             Roll your Arete to work an effect; successes are
                             its magnitude. +cast/vulgar flags Paradox risk.
  +quint                     Show/spend/gain Quintessence (capped by your Avatar).
  +paradox                   Show, accrue, or work off Paradox.
  +resonance                 List/set your Resonance traits.

**Werewolf (W20)** — you are Garou, of a Breed / Auspice / Tribe:

  +rage                      Show your Rage (temporary pool / permanent rating);
                             +rage/spend <n> for extra actions, +rage/gain <n>
                             to recover (only up to your permanent Rage),
                             +rage/frenzy to roll Willpower against frenzy.
  +gnosis                    Show/spend/gain your Gnosis (recovers up to your
                             permanent rating). Raise permanent Rage/Gnosis with
                             +xp/spend rage=<n> / gnosis=<n>.
  +renown                    Show Glory/Honor/Wisdom and your Rank;
                             +renown/gain <type>=<n> logs temporary Renown.
  +gifts                     List the Gifts you know.
  +gift/roll <gift>=<attribute>+<ability> [vs <diff>]
                             Build and roll the pool a Gift names.
  +shift <form>              Change form (homid/glabro/crinos/hispo/lupus); the
                             war forms' Physical bonuses apply to your rolls
                             while shifted.
  +rank                      Show your Rank (from permanent Renown).

## Staff

  @sheet <target>                    Staff view of a sheet (shows hidden fields).
  @sheet/set <target>/<field>=<value>
                                     Set any field directly (validated, audited).
                                     Setting <field>=template re-shapes the sheet
                                     to that splat.
