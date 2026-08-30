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
                    lifts one; bare @siteban lists. Wizard.

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

Powers (grantable to non-staff by Wizards, via @set): BOOT, EXAMINE_ALL,
TEL_ANYWHERE, TEL_ANYTHING, CHOWN_ANY, COMSYS_ADMIN, BUILDER_LLM,
SEE_HIDDEN. Wizards hold every power implicitly.

If the store goes down the game enters read-only mode automatically:
mutating commands pause with an alert to Wizards, and lift on recovery.
