# Jobs

The jobs system is the game's ticket tracker: player requests, character
notes, and the staff queues behind approvals (character generation, XP
spends, plot requests).

For players:

  +request <title>=<text>
                     File a request with staff: a bug, a question, a
                     plot idea, an appeal. You get a ticket number back;
                     staff answer on the ticket, and you are notified.
  +jobs              List the jobs you can see — you always see the ones
                     you opened or are assigned to.
  +job <n>           Read one of your tickets, with its comment trail.
  +note <char>/<category>=<text>
                     Attach a note to your own character (visible to
                     you and staff — background details, contacts,
                     reminders). +notes lists your own.

For staff:

  +jobs [<queue>]    List open tickets, optionally in one queue
                     (REQ, chargen, xp, ...).
  +job <n>           Read one ticket, with its comment trail.
  +job/assign <n>=<staffer>     Take or hand off a ticket.
  +job/comment <n>=<text>       Add a comment (player-visible).
  +job/approve <n>[=<comment>] / +job/deny <n>=<reason>
                     Resolve approval-type tickets. Approval handlers do
                     the real work: an approved chargen ticket marks the
                     character approved, an approved XP spend applies the
                     sheet change. A handler failure leaves the job (and
                     everything it would have written) untouched.
  +job/status <n>=<open|inprogress|resolved|denied>
                     Set the status directly (never runs approval
                     handlers; a resolved payload ticket cannot be
                     reopened — file a new one).

Notes:

- Ruleset modules register their own queues (chargen approval, XP) with
  a minimum staff role per queue.
- Tickets persist across restarts and are audit-logged.
