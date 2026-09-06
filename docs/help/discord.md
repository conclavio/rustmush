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
                      scene — no ambient room traffic. Applies from your next
                      follow.

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

Rules:

- One Discord user per account and one account per Discord user. To move a
  link, unlink first.
- The game stores your Discord user's numeric id (never your handle, which
  can change), the time you linked, your preferences, and the id of the DM
  conversation once a follow opened one. It never stores or logs what you say
  in Discord, nor what the DM showed you.
- Your Discord id is staff-only data. Staff cannot read it by rank: a
  specific, granted privilege is required, every look is recorded, and the
  record says only that a look happened — never what was seen.
- Refused links all read the same. If `/link` says the code could not be
  linked: check the code, mint a fresh one with `@discord/link`, and make
  sure neither side is already linked.

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
  @discord/unban <player>[=<reason>]
                               Or `@discord/unban <discord user id>` for a user
                               who unlinked while banned.

Every staff action here is recorded in the privilege audit
(`@privilege/audit`).

See also: help admin, help sshkey.
