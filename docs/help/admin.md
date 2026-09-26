# Admin

Staff tooling. Everything here is role- or power-gated; mortals get
"Permission denied." (or, for @stats, the short form).

People:

  @boot <player>[=<message>]
                    Disconnect all of a player's connections at once.
                    They see "You have been booted by <you>" plus your
                    message before the line drops. Needs the BOOT power
                    (Wizards implicitly). You cannot boot up-rank.
  @wall <message>   Broadcast "Announcement from <you>: <message>" to
                    every connected player. Wizard.
  @siteban <ip>[=<duration> <reason>]
                    Ban an IP from connecting; persisted across
                    restarts. Durations like 30m, 12h, 7d, or seconds;
                    no duration means permanent. @siteban/clear <ip>
                    lifts a persisted siteban; @siteban/lift <ip> pardons
                    prior password failures (IPv6 /64 by default), leaving
                    sitebans intact. Bare @siteban lists sitebans. Wizard.

Introspection:

  @ps               Scheduler inspection: pending @wait entries (who,
                    seconds remaining, command preview), module ticks
                    with cadence and next fire, totals. Wizard.
  @stats            Full server report for Wizards: uptime, object
                    counts by kind, connected players, world-loop lag
                    (last/max), store health (read-only mode, age of the
                    last write), db size. Mortals see the classic
                    "The universe contains N objects."
  WHO               Staff additionally see player locations.
  examine <object>  Controllers (and EXAMINE_ALL holders) see
                    everything: attributes, locks, flags, powers.

Server lifecycle:

  @dump             Trigger a background database snapshot (BGSAVE),
                    ordered behind pending writes; confirmation is
                    ack-gated on the store. Wizard.
  @shutdown         Graceful stop: announce, stop the world, drain all
                    pending writes, snapshot, exit. Wizard.

Hot restart (split deployment): on a server that runs the forked
gateway/server split, a deploy restarts the GAME server WITHOUT dropping
your connection. You will see "GAME: The server is restarting; hold on"
then "GAME: Reconnected" and a fresh look at your current room — you are
not disconnected and do not log in again. During the brief gap commands
are refused with that notice rather than lost. What DOESN'T survive the
restart (re-do it afterward): an unsubmitted +chargen wizard or mail
compose you were mid-typing, any pending @wait timers, an LLM action
(@gen / NPC reply) that was still generating, and any ACTIVE SCENE plus its
live per-scene ruleset state (e.g. combat heat) — re-open the scene after
reconnect. Everything you had already saved — rooms, your sheet, sent mail,
jobs — is intact. Operators: see docs/ops.md "Hot-restart (split) deployment".

The simulator's black box (SIM_ADMIN or Royalty):

  @arena/report [match] [chunk]
                    Read a match's tape itself: sixteen chunks a request
                    from the chunk named (0 by default), one line an event
                    in arena time, with object numbers, contact ids and
                    the draws; the position summaries are left out. The
                    header carries the outcome, the seed, the tape's
                    digest and chunk count, and every unit with its pilot.
                    Bare, the newest match the centre remembers; a running
                    match, a crashed one and one older than the last
                    twenty are read too. A page shows at most 200 lines
                    ("... and N more of this page." past them), and its
                    "Next: @arena/report 12 16" pages on from the chunk
                    the page was cut short in, so following it skips
                    nothing. A pilot's own debrief is SIM REPORT (help
                    simulator), which never carries any of this.
  @arena/tempo <match>=<period_ms>|off
                    Floor the wake interval of a running match, in steps
                    of 500 ms up to 10 s: the arena wakes once per period
                    of wall time and moves at most max_catch_up_steps
                    periods of its own tempo per wake, which is slow
                    motion when the floor is longer than that. Pilots'
                    orders wait for the next wake. Never a pause, not
                    hashed, recorded on the tape (tempo_floor). "=off"
                    lifts it; "@arena/tempo 12" reads it, and
                    @arena/status shows "tempo floor: 5000 ms" while one
                    stands. Nobody but you is told.
  @arena/place <unit>=<x>,<y>
                    Put a unit of a running match on an arena cell, as
                    the unit's own seat. The world refuses at once, and
                    never on the tape, a match that is not running
                    ("Match 12 is stopping.") and a cell off the window
                    or on impassable ground. The arena, at its next
                    pulse, refuses an occupied cell or a round in flight
                    to or from the unit, and every place it receives is
                    on the tape: placed, or place_refused with the
                    reason (stopped when an abort or another stop ended
                    the match in that pulse). Only a place in the very
                    pulse a kill or the time limit ends the match leaves
                    no line. A wreck and a linkdead pilot's tank are
                    placed like any other. Its pilot is told nothing,
                    and every other pilot's sensors simply find it where
                    it now is.
  @arena/watch <match>|off
                    Watch a running match: its whole field reaches your
                    panel on arena.observer at most once an arena second
                    - every tank by callsign, cell, heading and object
                    number, whether it is powered, held, a wreck or the
                    machine's, and how many rounds are in flight - and
                    screen 0 of the TUI and the web draws it live; the
                    line client prints none of it. For a staffer with no
                    centre slot and no seat ("You are flying, or about
                    to; EXIT and leave your slot before you watch." -
                    any character of a flying account, and a pilot who
                    has EXITed and kept the slot, too); at most eight a
                    match. "off" stops it, and so do a disconnect, the
                    end of the match and losing the power; bare says
                    what you watch. While you watch, SIM REPLAY answers
                    "You are watching match 12; @arena/watch off first."
                    No pilot is ever sent any of it.

Powers: BOOT, EXAMINE_ALL, TEL_ANYWHERE, TEL_ANYTHING, CHOWN_ANY,
COMSYS_ADMIN, BUILDER_LLM, SEE_HIDDEN, FACTION_ADMIN, SIM_ADMIN. Wizards
hold every power implicitly. (There is no in-game command to grant a power
to a non-staff object yet.)

Privileges are the opposite of powers in one way: NO rank implies one — not
Wizards, not God. A privilege exists only as an explicit grant, and the
grant, the revocation, every permitted use and every refused attempt are
recorded in a durable audit that never records what was seen.

  @privilege/grant <player>=<privilege>=<reason>
                    Grant a privilege (Wizards). The reason is required.
  @privilege/revoke <player>=<privilege>=<reason>
  @privilege/list [<player>]
                    Every holder, or one player's privileges.
  @privilege/audit [<player>]
                    The recent audit, newest first; with a player, only the
                    entries where they acted or were the subject.
  @privilege/archive [<page>]
                    Older audit history, a page at a time (0 = newest).

Privileges: DISCORD_IDENTITY — read a player's linked Discord identity with
@discord/show (see "help discord").

If the store goes down the game enters read-only mode automatically:
mutating commands pause with an alert to Wizards, and lift on recovery.
