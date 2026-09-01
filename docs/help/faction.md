# Factions

Factions are first-class organizations: sects, clans, coteries, gangs, crews,
corps, houses, and merc units. The engine's registry is the **truth** about
them — who belongs, at what rank, who they answer to, what they hold — and
that truth drives authorization (intel clearance, channel eligibility). But
**what you see is a projection**: every membership, structural tie, and
territorial hold carries a visibility policy, and the game shows you only
what your character could plausibly know. A covert member looks like a
non-member. A hidden faction looks like no faction at all.

## Player commands

  +factions           The factions you know of, with their publicly known
                      member counts (and your own membership, if any).
  +faction <name>     What you know about one faction: its charter, known
                      structure ("Chapter of the Camarilla"), known members
                      and their titles, known holdings, and your own standing.
                      Officers of a faction also see member levels and the
                      faction's resource pools.
  +territory [name]   The territory map as you know it: each region and who
                      is known to hold it. A region can carry several kinds
                      of hold at once — a **de jure** claim (the Prince still
                      claims the waterfront) alongside a **de facto** hold
                      (the gang actually runs it), with contests marked.
                      Covert holds show only to the holder's officers.
  +intel              Includes your factions' shared intel pools when your
                      rank clears you to read them — see `help intel`.

Membership, level, and title are set by staff. Levels are an authorization
scalar: reaching the faction's **officer** level lets you see more (levels,
resources, officer-known facts); reaching its **intel** level lets you read
the faction's shared intel pool.

## Visibility (what "known member" means)

Every membership has one of four visibilities, set by staff:

  public       Anyone can see it.
  members      Only fellow members see it.
  officers     Only the faction's officers see it.
  covert       Only you and staff know. Deep cover: even your faction's
               officers cannot see the record.

The same ladder applies to structural ties between factions and to
territorial holds. Secrets are real: the engine knows the Setite is not
really Toreador, and nothing on any display surface will tell you otherwise.
Discovery happens in play.

## Staff commands (Royalty+ or the FACTION_ADMIN power)

  @faction                    List all factions (truth).
  @faction/create <name>      Create; /destroy removes it and cascades
                              (memberships, inbound ties, control records).
  @faction/desc <f>=<text>    Set the charter.
  @faction/set <f>=<field>:<value>
                              hidden (yes/no), officer (level), intel
                              (level), aliases (a,b), ext (raw JSON for the
                              ruleset extension).
  @faction/add <f>=<player>[/level:<n>][/vis:<v>][/title:<t>]
  @faction/mod <f>=<player>[/level:<n>][/vis:<v>][/title:<t>]
  @faction/remove <f>=<player>
  @faction/edge <from>=<kind>/<to>[/vis:<v>][/note:<text>]
                              Structural ties: chapter, subordinate,
                              contracted, front. Front edges default covert
                              (the public face is real; the tie is the
                              secret). Ties are acyclic per kind and never
                              confer membership or permissions by themselves.
  @faction/unedge <from>=<kind>/<to>
  @faction/principal <f>=<npc>   Toggle an NPC voice of the faction.
  @faction/resource <f>=<kind>/<delta>[/<reason>]
                              Typed resource pools (influence.police, nuyen,
                              boon_credit — whatever the game defines), every
                              change cause-stamped.
  @faction/channel <f>        Show the current binding.
  @faction/channel <f>=<channel>
                              Bind a comsys channel: PUBLIC members are
                              auto-joined; quieter visibilities never are — a
                              channel roster is readable by anyone passing
                              its see lock (@channel/who), so it must not
                              carry what the registry hides. Lock the
                              channel's see/join (@channel/perm) or its
                              roster and traffic are world-open; the bind
                              warns you if they are. `<f>=` (explicit '=')
                              unbinds. The module never removes a channel
                              membership it did not create.
  @faction/show <f>           The full truth dump.

  @region/create <name>       Territory regions. A region may cover zone
  @region/zone <r>=<zmo>      master objects — or nothing at all: "Media
  @region/note <r>=<text>     influence in Chicago" is a perfectly good
  @region                     region with no rooms behind it.
  @faction/control <region>=<f>/<kind>[/strength:<n>][/vis:<v>]
                              Kinds: dejure, defacto, covert, economic.
  @faction/uncontrol <region>=<f>/<kind>
  @faction/contest <region>=<holder>/<kind>/<challenger>   (toggle)

In this first cut control records are staff-set. The faction *simulation* —
fronts and clocks that advance from scene outcomes and flip control
deterministically — arrives with the canon ledger.

## NPCs reporting to factions

An NPC informant's principal can be a faction instead of a person:

  @agent/set <npc>/principal=faction:<name>

What it perceives then lands in the faction's shared intel pool, readable via
`+intel` by members whose level passes the faction's intel gate. The faction
thereby *knows* things — and can be wrong, misled, or slow, because its
knowledge is whatever was reported, never the server's omniscient truth.
See `help intel`.

## See also

  +factions, +faction, +territory, +intel, help intel, help npc
