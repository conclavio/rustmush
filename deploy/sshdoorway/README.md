# sshdoorway deploy (doc 29)

sshdoorway is the port-22 SSH front door. Players type `ssh dev1+faenix@conclav.io`
and the doorway routes them to dev1's **trusted ingress**, a Unix socket, passing on
their real address and any key they proved they hold. Design:
[doc 29](../../docs/design/29-sshdoorway.md). Build plan:
[doc 29a](../../docs/design/29a-sshdoorway-implementation-plan.md).

**Installed on conclav.io on 2026-10-31 (cut DW.4).** Admin `sshd` moved to port 2222
first (the box's ssh.socket is socket-activated; `Port 2222` in
`/etc/ssh/sshd_config.d/10-ports.conf`, honored through Ubuntu's
`sshd-socket-generator`), which freed 22. The doorway listens on
`31.220.52.183:22` and `[2a02:4780:10:ccaf::1]:22`; its host key is
`SHA256:MWrhOxRvUJTnDrimeJJRT7kOhM27gX5TCqG8XS49Nng` (ed25519). A client that
had admin sshd's key for `conclav.io` (port 22) in `known_hosts` sees "host
identification changed" once: `ssh-keygen -R conclav.io`, then accept the
fingerprint above. The instances' own keys on 6789/6792 are untouched.

## Files

| File | Installed as |
|---|---|
| `sshdoorway.toml` | `/etc/rustmush/sshdoorway.toml` (edit `[listen].addrs`) |
| `sshdoorway@.service` | `/etc/systemd/system/sshdoorway@.service` |
| `99-sshdoorway.conf` | `/etc/sysctl.d/99-sshdoorway.conf` (`tcp_migrate_req = 1`) |
| `sshdoorway.conf` | `/etc/tmpfiles.d/sshdoorway.conf` (`/run/rustmush-ssh` setgid, the log directory) |
| `logrotate.conf` | `/etc/sshdoorway/logrotate.conf` |
| `sshdoorway-logrotate.service`, `.timer` | `/etc/systemd/system/` (rotation every 15 minutes) |
| `check-logging.sh` | `/opt/rustmush/sshdoorway/check-logging.sh` (the ship-time gate) |
| `swap.sh` | `/opt/rustmush/sshdoorway/swap.sh` (a thin call into the shared script below) |
| `../drain/swap.sh` | `/opt/rustmush/drain/swap.sh` (the swap mechanism shared with the gateway, doc 33 GG.3) |

## First install (outline for DW.4)

1. **User and binary.**
   - `useradd --system --no-create-home --shell /usr/sbin/nologin sshdoorway`.
   - Build with `cargo build --release -p mush-sshdoorway` and copy
     `target/release/sshdoorway` to `/opt/rustmush/sshdoorway/`.
   - `apt-get install zstd`.
2. **System files.** Install every file in the table above. Then run
   `sysctl --system`, `systemd-tmpfiles --create` and `systemctl daemon-reload`.
3. **Each instance.** Set `TRUSTED_SOCKET=/run/rustmush-ssh/<instance>.sock` in its
   `instance.env` (the template renders it into `[ssh].trusted_socket`; empty means
   no doorway on this box), then `GATEWAY_SWAP=1 bash .../dev<N>-split/deploy.sh`:
   the render carries the key, and the swap starts the idle gateway generation,
   which binds `<instance>.<gen>.sock` and makes the `<instance>.sock` symlink the
   doorway dials (doc 33 §4.4). Never a plain `systemctl restart`, which drops
   players. The tmpfiles directory and the `sshdoorway` group must exist first or
   the new generation's trusted bind fails and the swap rolls back. The gateway creates the socket with mode
   0660. Because the directory is setgid, the socket's group is `sshdoorway`, and only
   the doorway can connect to it. The gateway also checks each connection's peer
   credentials: it serves only a peer whose primary group is the socket's group (the unit
   runs the doorway as `Group=sshdoorway`), so not even the gateway's own user can forge a
   preamble. The gateway units already carry `ReadWritePaths=-/run/rustmush-ssh`, which they
   need under `ProtectSystem=strict`. The instance's public port (6789, 6792) keeps working
   (Q-29-4).
4. **Check the config.** Run `sshdoorway --config /etc/rustmush/sshdoorway.toml --check`.
5. **Start.** Run `systemctl enable --now sshdoorway-logrotate.timer`, then
   `systemctl enable --now sshdoorway@a`. Exactly one generation is enabled at any time;
   `swap.sh` moves the enablement to the generation it starts, so a reboot brings back the
   serving one.
6. **Gate.** `check-logging.sh` must pass. Do not tell players about port 22 until it
   does. Two of its checks need time or traffic, not fixing: the timer must have
   fired once (it runs at :00/:15/:30/:45), and each running generation must have
   logged at least one line since the last rotation (a connection will do), or the
   `notifempty` rotation has nothing to archive and the run fails.
7. **Publish the host key fingerprint.** It is logged at start as `fingerprint=` (Q-29-5).

## Restarting without dropping players

Run `swap.sh`. It:

1. starts the idle generation and waits for it to report ready;
2. after checking the new generation stayed up for 5 s, sends `SIGUSR1` to the old
   generation, which stops accepting connections. With
   `tcp_migrate_req = 1`, connections already queued on it move to the new generation.
3. lets the old generation keep serving its sessions, warning each player when 12h, 6h,
   2h, 1h, 15m and 1m remain;
4. closes the old generation once its last session ends, or at 24 h.

`swap.sh` only names the doorway's units, marker directory and checks; the mechanism is
`deploy/drain/swap.sh`, shared with the gateway (doc 33 §4.5). It looks for the shared
script at `../drain/swap.sh` (hence `/opt/rustmush/drain/`), or where `$DRAIN_SWAP` says.

`swap.sh` refuses to start a third generation while one is still draining. With
`swap.sh --force`, the draining generation is finished instead: one 30 s warning, then
close.

**`systemctl stop` or `restart` is not a swap.** `SIGTERM` finishes the generation: one
30 s warning, then every session closes. That is what a reboot does too. Use `swap.sh` to
restart without dropping players.

**A drained generation stays drained.** The drain writes `/var/lib/sshdoorway/draining-<gen>`,
and the doorway never removes it: a generation that finds its marker at start logs why,
reports ready and exits cleanly, whether systemd restarted it after a crash, needrestart
restarted it, or the box rebooted. Only `swap.sh` clears the marker, right before it starts
that generation. If `swap.sh` sees that a new generation did not stay up for 5 s, it stops
and disables that generation again, so `Restart=on-failure` cannot bring it up beside the
old one.

**Reloading.** `systemctl reload sshdoorway@a` (`SIGHUP`) reloads the routes, limits and
menu. Changing `[listen]` needs a swap.

**Your own addresses.** Put the networks you run bots or staff sessions from in
`[limits].exempt_nets` (CIDR or bare addresses; loopback is there by default). They skip
the per-IP connection, rate and /48 caps, not the global cap. A reload applies the change.

## Logs

- **The file.** `info` and above goes to `/var/log/sshdoorway/<generation>.log`.
  logrotate checks every 15 minutes. It rotates at 20 MB or daily, compresses with
  zstd, and keeps 60 archives or 14 days, whichever is fewer.
- **journald.** It gets `warn` and above, rate-limited to 200 lines every 30 s.
- **What is never logged.** Passwords, and anything after the first `+` of a username.
