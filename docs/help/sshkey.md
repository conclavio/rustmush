# SSH Keys

Register an SSH public key on your account and you can log in with no
password at all: `ssh -i <your key> <account>@game` drops you straight in,
and `ssh -i <your key> <account>+<character>@game` logs you directly into
one of your characters (skipping the character-select menu).

Login is by ACCOUNT, the same as the web and the password prompt. A key you
register authorizes that whole account — every character on it.

Commands (they act only on YOUR OWN account):

  @sshkey/add <public key line>
                    Authorize a key. Paste the ONE line from your PUBLIC
                    key file (`~/.ssh/id_ed25519.pub` or similar) — it
                    starts with `ssh-ed25519`, `ssh-rsa`, or `ecdsa-...`.
                    The comment at the end is kept only for display.
  @sshkey/list      Show your registered keys: an index, the SHA256
                    fingerprint, the key's comment, and when it was added.
  @sshkey/del <index-or-fingerprint>
                    Remove one key, named by its @sshkey/list index (e.g.
                    "@sshkey/del 2") or by its full "SHA256:..."
                    fingerprint.

Public vs private — read this once:

  A key comes in a PAIR. The PUBLIC half (the ".pub" file, one line
  starting with ssh-ed25519/ssh-rsa/ecdsa) is what you register here — it
  is safe to share. The PRIVATE half (no ".pub", often begins with
  "-----BEGIN OPENSSH PRIVATE KEY-----") is a secret you NEVER paste or
  send to anyone. @sshkey/add refuses a private key on sight and never
  echoes, logs, or stores what you typed, but the safe habit is to only
  ever copy the ".pub" file. On your own machine:

      cat ~/.ssh/id_ed25519.pub          # this is what you paste
      ssh-keygen -t ed25519              # run first if you have no key yet

How it takes effect: a key you add works on your very NEXT connection —
there is nothing to restart. A key you remove stops working immediately;
password login (`connect <account> <password>`, or the SSH password
prompt) always remains available as a fallback.

Up to 10 keys per account. Re-adding a key you already have (even under a
different comment) is a harmless no-op.

See also: "help getting-started", "help admin".
