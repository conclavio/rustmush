# Weather

Zones can have live weather: a deterministic state machine (clear, overcast,
fog, rain, storm, snow, ...) driven by a climate's seasonal transition odds,
ticking quietly in the background. Rooms opt in with their flags, and the
in-character clock (see `help time`) drives the seasons the machine follows.

For players, weather is just something you read. When you `look` in an
outdoor room you'll see a current-conditions line ("Overhead: steady rain,
around 9C, visibility poor."), and now and then a line of weather ambience
drifts by. The web client also carries a small weather panel.

Room flags decide what a room shows:

  OUTDOORS    Full weather: conditions on look, transition lines when the
              weather changes, and occasional ambient lines.
  SHELTERED   Muffled: it hears state changes ("Through the windows...") but
              no ambient drip.
  (neither)   Indoor: silent. An indoor room in a weather zone shows nothing.

Staff commands (Wizard/Royalty) — `<zone>` is `here` (this room's zone) or an
object match, usually the Zone Master Object's dbref:

  @weather <zone>                 Show a zone's current conditions.
  @weather <zone>=<state>         Pin a state (pauses the machine and
                                  broadcasts the change to the zone's rooms).
  @weather/release <zone>         Unpin; the machine resumes from that state.
  @weather/climate <zone>=<name>  Assign a climate and opt the zone in.
  @weather/climate <zone>=none    Opt the zone out of weather.
  @weather/list                   List every weather cell.

Setting a zone up, start to finish:

  @create Downtown              (or use an existing Zone Master Object)
  @set Downtown=ZONE_MASTER
  @zone here=Downtown           put a room into the zone
  @set here=OUTDOORS            let its weather show
  @weather/climate here=temperate-coastal

Shipped climates: temperate-coastal, desert, arctic. Weather state is always
deterministic; only the flavour text varies, drawn from the climate's line
pools.

## LLM flavour top-up (Royalty, needs `BUILDER_LLM`)

The state machine (which weather happens, and when) is **always deterministic**
— the LLM never touches it. What you *can* top up is the pool of **flavour
lines** the machine draws from: the ambient drift and the transition lines. Each
generated line lands **pending** and enters play only after staff approval, so
nothing the model writes ever reaches players unreviewed.

  @weather/generate [<climate>] [<state>]   Request flavour variants for a bucket.
                                            No args = every in-use climate/state;
                                            one = that climate; two = one bucket.
  @weather/lines <climate> <state>          List built-in / pending / approved lines.
  @weather/lines/approve <id>               Move a pending line into the live pool.
  @weather/lines/reject <id>                Discard a pending line.

`@weather/generate` never blocks: it fires one cheap off-loop call per
`(climate, state)` bucket (metered and audited like all LLM calls — see
`help llm`) and the pending lines arrive a moment later. How many variants it
asks for per bucket is `[weather].flavor_variants` (default 10, 64 maximum;
zero disables generation). A generated pool holds at most 256 lines; approved
lines are never evicted automatically, so reject obsolete lines before topping
up a full pool. Start to finish:

  @weather/climate here=temperate-coastal
  @weather/generate temperate-coastal rain     (pending lines arrive shortly)
  @weather/lines temperate-coastal rain
  @weather/lines/approve <id>                   (now it can appear in play)
