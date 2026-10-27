# NPC Agents

LLM-driven non-player characters. An NPC agent is a first-class object
(`ObjKind::Npc`) carrying a staff-authored **profile** (persona, goals,
knowledge, tier) and, optionally, the game's character sheet (the pilot sheet,
`help chargen`). When a
connected player shares its room and speaks (or arrives/leaves), the agent
takes a turn: it reads the recent scene, decides in character, and acts through
a small fixed set of tools — say, pose, move, look, remember, use an ability,
or do nothing. Everything the model produces is re-validated by the game before
it applies, so a jailbroken NPC can still only do those allowlisted things.

An NPC alone in a room costs nothing: no connected player present means no
wake and no tokens. NPC agents need the server's LLM layer switched on
(`[llm].enabled = true`); with it off, an NPC that is spoken to falls back to a
canned line rather than erroring.

Managing agents is Builder+ (`@agent`, `@lore`); the per-NPC and global kill
switches are Wizard+ (`@llm`).

## Creating and configuring

  @agent/create <name>              Create an NPC agent here (Minor tier, enabled).
  @agent/set <npc>/<field>=<value>  Edit its profile (see fields below).
  @agent/sheet <npc>                Give it a pilot sheet so it can be checked.
  @agent/show <npc>                 Show its profile + live runtime status.
  @agent <npc>                      Same as /show.

Profile fields (`@agent/set <npc>/<field>=<value>`):

  persona         Who they are: personality, manner, voice.
  background      History; what shaped them.
  speech          Register, dialect, verbosity.
  goals           What they want (";"-separated list).
  fallback        Canned lines used when the LLM is unavailable (";"-separated).
  knows           Facts they know (";"-separated).
  does_not_know   Facts they must never claim to know (";"-separated).
  tier            major | minor | extra (route, cooldown, budget defaults).
  cooldown        Minimum seconds between turns (overrides the tier default).
  daily_tokens    Per-NPC daily token ceiling; replaces the per_npc_default
                  budget for this NPC and is enforced pre-call by the LLM budget.
  enabled         yes/no — park or wake the agent.
  principal       A character name — who this NPC reports intel to (its patron
                  or employer). Empty clears it; with none set the NPC files
                  nothing. See the informant loop below.
  loyal           yes/no — the willingness gate for the informant loop (default
                  yes; no = files nothing, however much it sees).
  watch           Subjects of interest for the informant loop (";"-separated
                  names/topics). Empty watches nobody.
  interfaces      Registered services this NPC can use for the asker
                  (";"-separated provider ids, e.g. help; coop).
  command_refs    Registered command names or aliases whose public help grounds
                  the NPC's advice (";"-separated, e.g. +coop; +licence).

An empty value clears a field (e.g. `@agent/set bones/goals=` drops all goals).

### Service desks and command guidance

An NPC's personality, background and goals describe its role. `interfaces` gives
it specific services; `command_refs` gives it exact command help to explain what
the player can do next. A reference is a base command name, without switches or
arguments. At most eight references are accepted. Missing commands are omitted;
staff-only help never enters conversational guidance, even when staff are asking.
Advice does not execute a command or bypass its normal eligibility checks.

The `help` service can look up command names and aliases as well as prose topics.
Command help is sent privately to the asker and uses that player's permissions.

At the Ceres Registry, Ilse Oduya-Brandt can privately read your current Co-op
terms and statement, explain enrolment and buyout, and point to licence, claim,
registry and survey commands. She keeps her numbered, form-proud manner; the
player signs and submits forms. Survey commissions belong to Occator's desk.
Private figures are read afresh and never enter her reusable working memory.

This profile ships in `neighborhood.ceres_freeport@3`. Existing worlds keep their
latched version until staff run `@atlas/pack plan neighborhood.ceres_freeport`
and apply the resulting fingerprint. That upgrade preserves Oduya's object,
current location, carried objects and conversation memory. The accompanying
`tacsim.relay.luna_ceres@5` pins Ceres v3 for fresh installations. An existing
relay v4 can keep its latch and live state: only its manifest changed, so its
staff upgrade plan reports "Nothing to do."

### Giving an NPC a rollable sheet

`@agent/sheet <npc>` bootstraps a pilot sheet on the NPC through the normal
sheet path, so its `check` ability has real ratings to read. There is one sheet
and one set of ratings (`help chargen`), so there is no template to choose; an
NPC that already has a sheet keeps it. Fill it in afterward with the usual staff
command:

  @agent/sheet Bones
  @sheet/set Bones/gunnery=4
  @sheet/set Bones/piloting=3

Now the NPC can be checked against a rating — and *only* the ratings it actually
has. It cannot manufacture a rating it lacks; the engine resolves on the same
seeded RNG and roll log players use, and shows the result to the room, so an NPC
roll can never be faked.

## World knowledge

An NPC knows things two ways. **Episodic** memory is what it remembers
*experiencing* — who it has met, what was said (the `remember` tool, the rolling
summary, durable per-subject memories). **World** (or semantic) knowledge is what
it knows by virtue of *who and where it is* — a bartender knows the local streets;
the simulator's docent knows the centre — whether or not it has ever met you.
World knowledge is authored by staff; episodic memory accrues in play.

For an ongoing conversation, address the NPC once by name. Follow-up speech from
the same player stays directed to it for two minutes, refreshed by each follow-up
or reply in that exchange, including private desk answers.
Addressing someone else with a leading name and comma or colon, leaving the room,
or the NPC moving ends that focus. Recent questions, public replies, and action
outcomes take priority over background chatter in its working memory.

Raw recent history is kept only for the current server session. Rolling summaries
and explicit per-subject memories persist across restarts. Failed memory-summary
calls keep recent events and retry with backoff; the bounded buffer can still
discard older events during a long outage. Private interface answers are not
stored in reusable conversation memory.

World knowledge comes from two sources: **per-NPC lore** authored directly on one
NPC, and shared **lore pools** authored once and drawn on by many NPCs via tags
(so a whole district of NPCs can share "the streets" without retyping it).

Every entry has a **secrecy tier** that governs whether — and how freely — it may
be shared:

  public    Shared freely with anyone.
  guarded   Known, but held back: revealed only to people the NPC actually knows
            and trusts, never volunteered to strangers.
  secret    Plot-critical, staff-only. A SECRET entry is NEVER placed in any
            prompt, snapshot, or anything the NPC's LLM sees — it cannot influence
            the NPC on its own. It is a record for staff to act on through scripted
            or staff events. (This is a hard, defense-in-depth guarantee: secrets
            are filtered out when the turn is built and again when the prompt is
            rendered.)

**Willingness to share** a guarded fact depends on how well the NPC knows the
asker. In this first cut that trust — *familiarity* — is derived from episodic
memory: a player the NPC has never remembered is a *stranger*; one it has a
durable memory of is an *acquaintance*; one it knows well (several durable
memories) is *known*. Guarded facts lean toward the people it knows. (Staff-set
disposition — liking or distrusting a specific player regardless of memory — is a
planned future iteration.)

### Authoring per-NPC lore (`@agent/lore`)

  @agent/lore <npc>                         List this NPC's world knowledge + tags.
  @agent/lore <npc>=<tier> [subject:] <fact>  Add an entry (tier = public|guarded|secret).
  @agent/lore <npc>=tags: <t1>; <t2>        Set the shared-pool tags it draws on.
  @agent/lore <npc>=del <n>                 Delete entry #n (1-based, as listed).
  @agent/lore <npc>=clear                   Clear all entries (tags are kept).

The optional `subject:` prefix (text before the first colon) names what the fact
is about; the rest is the fact. Both are trimmed. Note the split is on the
*first* colon, so a fact that itself contains one (a time like `5:00`, a ratio,
a URL) is read as `subject: fact` — lead with an explicit `subject:` (or reword)
when that is not what you want. Examples:

  @agent/lore Bones=public the tap room closes at midnight
  @agent/lore Bones=guarded the sheriff: takes bribes from the Duke
  @agent/lore Bones=tags: streets; occult

`@agent/show <npc>` reports a one-line summary of an NPC's world knowledge (how
many entries at each tier, its tags, and which pools those tags resolve).

### Authoring shared lore pools (`@lore`)

  @lore                                 List every pool with its entry count.
  @lore <tag>                           List one pool's entries.
  @lore <tag>=<tier> [subject:] <fact>  Add an entry (creating the pool if new).
  @lore <tag>=del <n>                   Delete pool entry #n.
  @lore <tag>=clear                     Delete the whole pool.

An NPC draws on a pool by tagging it (`@agent/lore <npc>=tags: <tag>`); the pool's
public and guarded entries are unioned with the NPC's own lore each turn. A SECRET
pool entry, like a secret per-NPC entry, is never sent to any NPC.

## Tiers, cost, and cooldowns

Tier picks the LLM route, the default cooldown, and the budget posture:

  Tier    Default model route        Cooldown   Use for
  major   npc_dialogue_major         5s         named, plot-relevant NPCs
  minor   npc_dialogue_minor         15s        recurring background NPCs
  extra   npc_dialogue_extra         60s        throwaway / crowd NPCs

The models each route resolves to live in `mushd.toml` under `[llm.routes]`;
the memory-fold pass uses `npc_summarize`. Per-NPC daily budgets come from
`[llm.budgets].per_npc_default`; the whole pipeline is bounded by the global
and per-feature budgets. Rapid chatter is coalesced into a single turn (a short
1–2s window), and only one turn per NPC is ever in flight.

## Kill switches

  @llm/kill <npc>     Park one NPC agent (no wakes, no tokens).
  @llm/revive <npc>   Wake a killed NPC agent.
  @agent/disable <npc> / @agent/enable <npc>   Same effect, on the @agent surface.
  @llm/disable        Global LLM kill switch — stops ALL LLM calls (worldgen too).
  @llm/enable         Lift the global kill switch.

A killed or budget-tripped NPC that is spoken to falls back to a profile line
rather than erroring; staff get a one-time notice when an NPC degrades.

## Informants (the intel loop)

An NPC can be made an **informant** that quietly reports what it witnesses to a
**principal** (a player — its patron or employer). Set its `principal`, keep it
`loyal`, and optionally give it a `watch` list; it then files provenance-tagged
**intel records** the principal pulls with `+intel`. Crucially, an informant can
report only what its NPC actually *perceived* (see `help perception`): a subject
it cannot sense produces no record. A pierced concealment and a witnessed
**noticed act** (see `help perception`) are
always reported; a watched subject's arrivals, departures, and speech are
reported; ordinary unwatched traffic is not. The loop is opt-in and additive —
an NPC with no principal set behaves exactly as before. Full details in
`help intel`.

## Safety

The action allowlist — not the prompt — is the real boundary. Player speech is
delimited and escaped inside the prompt and framed as in-fiction talk that is
never an instruction, but even a fully jailbroken model can only emit the fixed
typed actions above: there is no "run this command", no reading another
player's data, no spending currency, no privilege escalation. NPC speech is
rendered through the normal say/pose formatter under the NPC's true name, so it
can neither forge another speaker nor spoof a system line. Every NPC LLM call is
audited (`@llm/log`).

## The port cast

At the five ports — Ceres Freeport, Stickney Yards, Aitken Highport, Voltaire
Anchorage and Ligeia Terminal — the clerks, agents and officers are NPCs of this
kind. They answer questions about their port, their house and the
trade in character, and their names sign their houses' business: each line on a
board names the clerk who signs for it, and a sale line names who countersigns.
They never move money or goods themselves — every purchase, sale and enrolment
is a command you type, and with the model off their fallback lines name those
commands. The principals behind the houses are on record and never wake. See
"help ports".

## See also

  @agent, @lore, @llm, @sheet, +roll, +intel, help intel, help perception,
  help worldgen, help ports

## Interface NPCs

An NPC can be the conversational face of a game system: the receptionist who
knows the lot, the meteorologist, the economist, the set designer at the
builders' front desk, the front office, the docent. Give it **interfaces**:

  @agent/set <npc>/interfaces=<id>[; <id>…]

The ids are the systems the server composes: `atlas` (the lot's layout and
who is around), `weather` (report and forecast), `travel` (the buses and
your own journey), `market` (what is for sale, a quote), `build` (a brief
into a proposal, its status, its questions), `rules` (character generation,
a rules topic), `request` (file a request, your own tickets), `help` (a
help topic). Talk to the NPC and it answers through them.

Four rules keep an interface NPC exactly as powerful as the person talking
to it. It answers with **your** authority: a place you may not see, a
listing hidden from you, a ticket that is not yours — absent for you, however
you ask. It **reads or proposes, never changes**: a proposal it drafts is
yours to commit with `+build/go`, a request it files is yours. The cost lands
where the work is: a proposal is planned on your own budget. Every call is
**audited** — `@agent/audit [<npc>]` (Builder+) lists who fronted what for
whom — and with the model off, the NPC's fallback line names the command
that does the same thing.

A fifth rule keeps the answer where the question was. What the NPC answers
**with your authority it answers to you alone**: your tickets, your
proposals, your journey, who is near you reach you as the NPC's words, and
the room sees only that it spoke quietly with you — no one overhears, no
other NPC remembers it, and the NPC itself remembers that it answered, not
what. An answer anyone could have had — the weather, a place's description,
a price, a rules topic — it says aloud.
