# sshdoorway deploy (doc 29)

sshdoorway is the port-22 SSH front door. Players type `ssh dev1+faenix@conclav.io`
and the doorway routes them to dev1's **trusted ingress**, a Unix socket, passing on
their real address and any key they proved they hold. Design:
[doc 29](../../docs/design/29-sshdoorway.md). Build plan:
[doc 29a](../../docs/design/29a-sshdoorway-implementation-plan.md).

**Nothing here is installed on any box yet.** The rollout (cut DW.4) waits for two
things from the owner: moving admin `sshd` off the player addresses, and a go-ahead.

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
| `swap.sh` | `/opt/rustmush/sshdoorway/swap.sh` |

## First install (outline for DW.4)

1. **User and binary.**
   - `useradd --system --no-create-home --shell /usr/sbin/nologin sshdoorway`.
   - Build with `cargo build --release -p mush-sshdoorway` and copy
     `target/release/sshdoorway` to `/opt/rustmush/sshdoorway/`.
   - `apt-get install zstd`.
2. **System files.** Install every file in the table above. Then run
   `sysctl --system`, `systemd-tmpfiles --create` and `systemctl daemon-reload`.
3. **Each instance.** Add `trusted_socket = "/run/rustmush-ssh/<instance>.sock"` to its
   `[ssh]` section, then restart its gateway. The gateway creates the socket with mode
   0660. Because the directory is setgid, the socket's group is `sshdoorway`, and only
   the doorway can connect to it. The instance's public port (6789, 6792) keeps working
   (Q-29-4).
4. **Check the config.** Run `sshdoorway --config /etc/rustmush/sshdoorway.toml --check`.
5. **Start.** Run `systemctl enable --now sshdoorway-logrotate.timer`, then
   `systemctl start sshdoorway@a`.
6. **Gate.** `check-logging.sh` must pass. Do not tell players about port 22 until it
   does.
7. **Publish the host key fingerprint.** It is logged at start as `fingerprint=` (Q-29-5).

## Restarting without dropping players

Run `swap.sh`. It:

1. starts the idle generation and waits for it to report ready;
2. sends `SIGTERM` to the old generation, which stops accepting connections. With
   `tcp_migrate_req = 1`, connections already queued on it move to the new generation.
3. lets the old generation keep serving its sessions, warning each player when 12h, 6h,
   2h, 1h, 15m and 1m remain;
4. closes the old generation once its last session ends, or at 24 h.

`swap.sh` refuses to start a third generation while one is still draining. With
`swap.sh --force`, the draining generation is finished instead: one 30 s warning, then
close.

**Reloading.** `systemctl reload sshdoorway@a` (`SIGHUP`) reloads the routes, limits and
menu. Changing `[listen]` needs a swap.

## Logs

- **The file.** `info` and above goes to `/var/log/sshdoorway/<generation>.log`.
  logrotate checks every 15 minutes. It rotates at 20 MB or daily, compresses with
  zstd, and keeps 60 archives or 14 days, whichever is fewer.
- **journald.** It gets `warn` and above, rate-limited to 200 lines every 30 s.
- **What is never logged.** Passwords, and anything after the first `+` of a username.
