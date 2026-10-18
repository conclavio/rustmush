# Fronts and clocks (faction pressure)

Factions don't sit still. A **front** is something a faction is actively pushing
— seizing a district, exposing a rival, extracting an asset — and a **clock** is
its deterministic progress. When a clock fills, its consequence happens: a
territory changes hands, a threat lands, and the world's chronicle records it.

The point is that clocks advance from *play*. A clock can be set to watch for a
kind of outcome and tick forward whenever one happens involving the faction's
people — so a scene your crew fights through actually moves a rival house's
grip on a port, without staff updating a spreadsheet by hand.
That connection — player action to territorial consequence — is the whole
engine.

## Players

  +clocks             The faction pressure you know of: each clock you can see
                      and how full it is. You see clocks a faction lets its
                      members (or the public) see; secret clocks stay hidden.

You don't advance clocks directly — your scenes and actions do.

## Staff

  @front              List fronts. @front/create <faction>=<title> ;
                      @front/resolve <id>
  @clock              List clocks. @clock/create <front-id>=<title>/<segments>
  @clock/set <id>=<field>:<value>
                        on:<canon kind>   auto-advance when such a canon event
                                          is recorded involving the faction's
                                          members (e.g. on:scene.concluded)
                        vis:<v>           who sees it (public/members/officers/
                                          covert)
                        says:<text>       the chronicle line when it completes
                        control:<region>/<kind>   flip territory to the faction
                                          on completion (kind: dejure/defacto/
                                          covert/economic). The resulting hold
                                          inherits the clock's visibility — a
                                          covert clock makes a covert hold, seen
                                          only by the holder's officers and staff.
  @clock/advance <id>[=<n>]   advance by n (default 1). Completing a clock fires
                              its consequence: the canon line and any territory
                              flip.
  @clock/del <id>

A completed clock is frozen — it fires its consequence once. The consequence is
recorded in the chronicle (see help chronicle) at the clock's visibility, and an
officers-only clock's completion is shown only to officers, not to the whole
membership. `front.progress` is reserved for these consequence lines: a clock
cannot be set to watch it (that would let clocks cascade into one another).

Not yet: a per-faction budget on offscreen actions, timed (scheduled) ticks, and
rumor seeds that plant what NPCs have heard — those arrive with the belief
system.

## See also

  +clocks, +territory, +faction, help faction, help chronicle
