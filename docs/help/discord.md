# Discord

The game can bind your account to a Discord user, so the game's Discord bot
knows who you are. Today (D1) the link is the foundation: it is what later
lets you follow a scene from Discord, act from it, and speak on relayed
channels as yourself. Nothing is sent to Discord until you link, and nothing
about your character leaves the game until those later features arrive.

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
                      Attach a Discord follow when your last terminal or web
                      connection drops (arrives with the follow feature).

In Discord, the bot answers three commands, all private to you:

  /link <code>        Bind this Discord user to the account that minted the code.
  /unlink             Remove the link.
  /whoami             Which account this Discord user is linked to.

Rules:

- One Discord user per account and one account per Discord user. To move a
  link, unlink first.
- The game stores your Discord user's numeric id (never your handle, which
  can change), the time you linked, and your preferences. It never stores or
  logs what you say in Discord.
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
