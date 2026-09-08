# Discord

The game can bind your account to a Discord user, so the game's Discord bot
knows who you are. The link is the foundation; on it, a **follow** turns the
bot's DM into a lossy window on one of your characters — what they see, in
plain text, and what you type runs as them — so stepping away from the
terminal in the slow middle of a scene does not stop the scene. Later, the
same link lets you speak on relayed channels as yourself. Nothing is sent to
Discord until you link, and nothing until you follow.

Linking (from any of your characters):

  @discord            Your account's Discord link: linked or not, auto-follow,
                      relay character. Never shows a Discord identifier.
  @discord/link       Mint a one-time code, shown to you alone. In Discord,
                      send the game's bot a direct message:
                          /link ABCD-EFGH
                      within ten minutes. The code works once; never share it.
                      (Type it any way you like — lowercase, with or without
                      the dash.)
  @discord/unlink     Remove the link. `/unlink` in Discord does the same.
  @discord/follow     Ask the bot to follow this character in your DM — the
                      same as `/follow` there, handy just before you step away.
  @discord/unfollow   Detach the follow.
  @discord/set relay=<one of your characters>
                      Which of your characters speaks in relayed channels when
                      none is attached (default: the one you connected most
                      recently). relay=none restores the default.
  @discord/set autofollow=on|off
                      When your last terminal or web connection drops, the bot
                      attaches a follow (below) before the link-dead grace
                      ends, so a scene never loses you mid-turn.
  @discord/set quiet=on|off
                      A follow's DM shows only notices, questions and your own
                      scene — no ambient room traffic. Applies at once to a
                      follow in progress, and to every follow after.

In Discord, the bot answers these commands, all private to you:

  /link <code>        Bind this Discord user to the account that minted the code.
  /unlink             Remove the link.
  /whoami             Which account this Discord user is linked to.
  /follow [character] Follow one of your characters here (below).
  /detach             Stop following.
  /say, /pose, /look, /scene
                      Shortcuts for the game lines of the same name.

Following:

  /follow attaches one more connection to your character's session — beside
  your terminal or browser, or instead of them. Name the character if your
  account has several (your relay character is the default). The DM then
  shows what that character sees: speech and poses in the room, the room
  when you look, scene turns, pages and other notices, and questions the
  game asks you — as plain text, gathered every second or two. It is never
  more than your terminal would show, and usually less: panels, WHO, channel
  chatter and formatting are left out.

  Anything you type in the DM runs as your character — `say hi`, `:waves`,
  `+roll 3d6`, `respond 2`, `+scene/pause` — under exactly the same rules as
  a terminal (turn order, flood limits, @ignore, staff boots). `QUIT` or
  `/detach` stops following; your character disconnects as with QUIT unless
  another connection remains. Join from your terminal at any time: it is the
  same session, and the game announces one connection, not two.

  One follow per Discord user: `/follow` for another character replaces the
  first. A follow with no DM input for six hours (by default) that is not in
  a scene is detached with a note; so is one whose DMs the bot can no longer
  reach. Under a burst the DM drops the oldest room lines first and says how
  many — notices and questions are never dropped.

Buttons:

  When the game asks you something — agree to end the scene, pick a target,
  spend a point — the question arrives as its own message with its answers
  as buttons (or a drop-down when there are many). Tapping one answers
  exactly as `respond <n>` would at a terminal; the message then shows your
  answer and the buttons go away. "Decline" is `respond cancel`. A free-text
  question offers "Answer…", which opens a small form. If a question times
  out unanswered, the message says so and the game applies its default — the
  same as at a terminal. The game only ever accepts the first answer, from
  whichever screen it came.

  A notice that can be replied to — a page, a job update — carries a reply
  button that opens a form prefilled with the command (`page Wiz=`); edit,
  submit, and the line runs as your character. Buttons on old notices stop
  working after a while; type the command in the DM instead.

  Turns: when a scene's turn comes to you (`+timestop/next`, or the scene
  resuming on your turn), you get a notice on every interface — the DM
  included. When the facilitator ends the scene, rewinds a turn, or seals
  its outcome, the "do you agree?" question arrives here as Yes/No buttons;
  when a nudge is set and your turn has sat idle, "Pass / Hold" does too
  (`help timestop`, Consent).

Relayed channels:

  Staff can mirror a comsys channel to a Discord channel and back
  (`@channel/set Public=relay`, plus the mapping in the server's config).
  Every line spoken on such a channel appears in the Discord channel under
  the speaker's character name; every message a linked Discord user posts
  in that Discord channel is spoken on the game channel as their character
  — the one they are following, else their relay character
  (`@discord/set relay=<character>`), else their first. The same rules as
  typing it apply: you must be on the channel, ungagged, and allowed to
  speak there; `:waves` and `;grins` pose as they do in game. A line from
  Discord is never mirrored back to Discord, and a channel is never relayed
  unless staff flagged it. Unlinked Discord users are told once in a while
  that their line was not relayed; banned users are ignored.

  Lines you type here run as your character exactly as in the game, so what
  you say in a relayed Discord channel is in the game's history like any
  other channel line.

Rules:

- One Discord user per account and one account per Discord user. To move a
  link, unlink first.
- The game stores your Discord user's numeric id (never your handle, which
  can change), the time you linked, and your preferences. It never stores or logs what you say
  in Discord, nor what the DM showed you.
- Your Discord id is staff-only data. Staff cannot read it by rank: a
  specific, granted privilege is required, every look is recorded, and the
  record says only that a look happened — never what was seen.
- Refused links all read the same. If `/link` says the code could not be
  linked: check the code, mint a fresh one with `@discord/link`, and make
  sure neither side is already linked.

Staff channel:

  When the operator has mapped a staff channel, the bot posts there (as the
  bot, never as a character) every `+request` a player files and every staff
  economy alert — a headline with the command to open it. Reply and act in
  the game as usual; a `Staff` comsys channel flagged `relay` gives you a
  two-way staff channel on Discord too.

Staff (Wizards):

  @discord/show <player>       The player's link, including the Discord id.
                               Requires the DISCORD_IDENTITY privilege
                               (`help admin`); recorded in the privilege audit.
  @discord/unlink <player>[=<reason>]
  @discord/ban <player>=<reason>
                               Refuse the player's linked Discord user: no
                               linking, following, or relaying until unbanned.
                               The ban follows the Discord user, not the
                               account, so unlinking does not lift it.
                               Unlinking or banning also stops any current
                               follow for any character on that account.
  @discord/unban <player>[=<reason>]
                               Or `@discord/unban <discord user id>` for a user
                               who unlinked while banned.

Every staff action here is recorded in the privilege audit
(`@privilege/audit`).

See also: help admin, help sshkey.
