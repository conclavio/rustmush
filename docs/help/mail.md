# Mail

@mail is the in-game letter system: persistent private messages that
wait for their recipient, unlike a page which is live-only.

Reading:

  @mail              List the messages in your current folder (unread
                     marked U, urgent marked !).
  @mail <n>          Read message number <n> (numbers come from the
                     current folder's listing).
  @mail/unread <n>   Flip message <n> back to unread.

Sending:

  @mail/send <players>=<subject>
                     Start composing a letter. You are dropped into
                     compose mode: type your message line by line, then
                     send with "--" on a line of its own. "~p" previews;
                     a lone ~q always aborts compose mode.
  @mail/quick <players>=<subject>/<body>
                     One-shot send, no compose mode.
  Add /urgent to either form to flag the message urgent.

Managing:

  @mail/delete <n>   Soft-delete a message to the hidden trash folder.
  @mail/purge        Empty the trash for good.
  @mail/file <n>=<folder>
                     Move a message into a named folder.
  @mail/folder [<name>]
                     Show or switch your current folder. /newfolder and
                     /rmfolder manage up to 16 folders of your own.
  @mail/reply <n>    Reply to the sender; /replyall answers everyone.
  @mail/fwd <n>=<players>
                     Pass a message on.

Notes:

- New mail is announced when it arrives and when you connect.
- Quotas apply: staff configure how much mail one player may store.
  Trashed messages do not count — delete then purge always frees space.
- One message may address at most 32 recipients and contain at most 512 body
  lines / 64 KiB of UTF-8 text (subjects are limited to 512 bytes).
- A player can lock who may mail them: @lock/mail me=<expression>
  (see "help locks").
- The web client shows mail as a panel with threads derived from
  reply links.
